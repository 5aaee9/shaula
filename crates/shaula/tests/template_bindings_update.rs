//! Spec 0038 integration coverage: the schema-driven bindings read
//! projection on revision GETs and the Update keep/replace merge with
//! structural validation, including fail-closed behavior when the target
//! artifact's bindings schema is absent.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod common;
#[path = "support/template_updates.rs"]
mod template_updates;

use axum::http::StatusCode;
use common::artifact_variants;
use template_updates::{Fixture, TestResult};
use tower::ServiceExt;

/// The k8s fixture bindings (`namespace` + `kubeconfig`) with a real
/// per-field sensitivity split: namespace is public, kubeconfig is a
/// protected credential.
const SPLIT_SCHEMA: &str = r#"{
    "type": "object",
    "additionalProperties": false,
    "required": ["namespace", "kubeconfig"],
    "properties": {
        "namespace": {"type": "string", "minLength": 1, "sensitive": false},
        "kubeconfig": {"type": "string", "minLength": 1, "sensitive": true}
    }
}"#;

const SECRET: &str = "secret-kubeconfig";

async fn fixture_with_schema() -> TestResult<(Fixture, String)> {
    let fixture = Fixture::new().await?;
    let (target, bytes) = artifact_variants::with_bindings_schema(SPLIT_SCHEMA)?;
    // The split-schema variant is a genuinely different archive from both
    // the base and the runtime-policy target the fixture seeds.
    assert_ne!(target, fixture.base_digest);
    assert_ne!(target, fixture.target_digest);
    assert!(fixture.engine_binary.is_file());
    fixture
        .store
        .store()
        .artifact_archive_put(&target, &bytes, 1_800_000_001_000)
        .await?;
    let mut upload =
        common::authorized("PUT", &format!("/api/v1/template-artifacts/{target}"), None);
    *upload.body_mut() = axum::body::Body::from(bytes);
    assert_eq!(
        fixture.app.clone().oneshot(upload).await?.status(),
        StatusCode::CREATED
    );
    Ok((fixture, target))
}

async fn revision_bindings(fixture: &Fixture, revision: i64) -> TestResult<serde_json::Value> {
    let response = fixture
        .app
        .clone()
        .oneshot(common::authorized(
            "GET",
            &format!("{}/revisions/{revision}", template_updates::PROFILE),
            None,
        ))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&axum::body::to_bytes(response.into_body(), 1 << 20).await?)?;
    Ok(body["bindings"].clone())
}

#[tokio::test]
async fn revision_read_projects_non_secret_values_and_presence_markers() -> TestResult {
    let (fixture, target) = fixture_with_schema().await?;
    let body = serde_json::json!({
        "artifact_digest": target,
        "engine_ref": "terraform",
        "bindings": {"namespace": "renamed-ns"}
    });
    let (status, _, _) = fixture.send(body, Some(&fixture.etag), None).await?;
    assert_eq!(status, StatusCode::ACCEPTED);

    // The updated revision exposes the non-secret value verbatim and the
    // secret as a presence marker only — never the credential bytes.
    let bindings = revision_bindings(&fixture, 2).await?;
    assert_eq!(bindings["namespace"], serde_json::json!("renamed-ns"));
    assert_eq!(
        bindings["kubeconfig"],
        serde_json::json!({"sensitive": true, "set": true})
    );
    assert!(!bindings.to_string().contains(SECRET));

    // The base revision still points at the seeded base artifact and its
    // `{}` schema fails every field closed.
    assert_eq!(
        fixture.revision(1).await?.artifact_digest,
        fixture.base_digest
    );
    let base = revision_bindings(&fixture, 1).await?;
    assert_eq!(
        base["namespace"],
        serde_json::json!({"sensitive": true, "set": true})
    );
    assert_eq!(
        base["kubeconfig"],
        serde_json::json!({"sensitive": true, "set": true})
    );
    assert!(!base.to_string().contains(SECRET));
    Ok(())
}

#[tokio::test]
async fn update_merges_keep_and_replace_semantics_per_field() -> TestResult {
    let (fixture, target) = fixture_with_schema().await?;
    // Sensitive field omitted entirely: kept verbatim server-side.
    let body = serde_json::json!({
        "artifact_digest": target,
        "engine_ref": "terraform",
        "bindings": {"namespace": "renamed-ns"}
    });
    let first = fixture.send(body, Some(&fixture.etag), None).await?;
    assert_eq!(first.0, StatusCode::ACCEPTED);
    let row = fixture.revision(2).await?;
    let stored: serde_json::Value = serde_json::from_str(row.bindings_json.as_deref().unwrap())?;
    assert_eq!(stored["namespace"], serde_json::json!("renamed-ns"));
    assert_eq!(stored["kubeconfig"], serde_json::json!(SECRET));

    // A new secret value replaces; the non-secret field is retained by
    // omission.
    let body = serde_json::json!({
        "artifact_digest": target,
        "engine_ref": "terraform",
        "bindings": {"kubeconfig": "rotated-secret"}
    });
    let (status, detail, _) = fixture.send(body, Some(&first.2), None).await?;
    assert_eq!(status, StatusCode::ACCEPTED, "{detail}");
    let row = fixture.revision(3).await?;
    let stored: serde_json::Value = serde_json::from_str(row.bindings_json.as_deref().unwrap())?;
    assert_eq!(stored["kubeconfig"], serde_json::json!("rotated-secret"));
    assert_eq!(stored["namespace"], serde_json::json!("renamed-ns"));
    Ok(())
}

#[tokio::test]
async fn empty_schema_target_only_accepts_all_keep_submissions() -> TestResult {
    // The seeded runtime-policy target carries the fixture's empty `{}`
    // bindings schema: no declared fields, so any submitted value is an
    // unknown field, while an empty (all-keep) object inherits the base.
    let fixture = Fixture::new().await?;
    let body = serde_json::json!({
        "artifact_digest": fixture.target_digest,
        "engine_ref": "terraform",
        "bindings": {"namespace": "x"}
    });
    let (status, _, _) = fixture.send(body, Some(&fixture.etag), None).await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let mut body = fixture.body();
    body["bindings"] = serde_json::json!({});
    let (status, _, _) = fixture.send(body, Some(&fixture.etag), None).await?;
    assert_eq!(status, StatusCode::ACCEPTED);
    let row = fixture.revision(2).await?;
    let stored: serde_json::Value = serde_json::from_str(row.bindings_json.as_deref().unwrap())?;
    assert_eq!(stored["kubeconfig"], serde_json::json!(SECRET));
    assert_eq!(stored["namespace"], serde_json::json!("actions-runners"));
    Ok(())
}

#[tokio::test]
async fn update_rejects_invalid_merged_sets_without_partial_writes() -> TestResult {
    let (fixture, target) = fixture_with_schema().await?;
    for bindings in [
        // Unknown field under additionalProperties: false.
        serde_json::json!({"namespace": "x", "rogue": 1}),
        // Wrong type for a non-secret field.
        serde_json::json!({"namespace": 123}),
        // Presence-marker echo is never an update instruction.
        serde_json::json!({"kubeconfig": {"sensitive": true, "set": true}}),
        // Non-secret null is not the keep sentinel and fails the schema.
        serde_json::json!({"namespace": null}),
    ] {
        let body = serde_json::json!({
            "artifact_digest": target,
            "engine_ref": "terraform",
            "bindings": bindings
        });
        let (status, _, _) = fixture.send(body, Some(&fixture.etag), None).await?;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bindings}");
    }
    // No revision was minted by any rejected request.
    assert!(fixture.revision(2).await.is_err());
    Ok(())
}

#[tokio::test]
async fn missing_required_secret_in_merged_set_is_rejected() -> TestResult {
    // The base revision carries both fields, but a target artifact whose
    // schema additionally requires a field the base never stored must
    // reject the merged set instead of admitting an incomplete revision.
    let fixture = Fixture::new().await?;
    let schema = r#"{
        "type": "object",
        "additionalProperties": false,
        "required": ["namespace", "kubeconfig", "cluster_ca"],
        "properties": {
            "namespace": {"type": "string", "sensitive": false},
            "kubeconfig": {"type": "string", "sensitive": true},
            "cluster_ca": {"type": "string", "sensitive": true}
        }
    }"#;
    let (target, bytes) = artifact_variants::with_bindings_schema(schema)?;
    let mut upload =
        common::authorized("PUT", &format!("/api/v1/template-artifacts/{target}"), None);
    *upload.body_mut() = axum::body::Body::from(bytes.clone());
    assert_eq!(
        fixture.app.clone().oneshot(upload).await?.status(),
        StatusCode::CREATED
    );
    fixture
        .store
        .store()
        .artifact_archive_put(&target, &bytes, 1_800_000_001_000)
        .await?;
    let body = serde_json::json!({
        "artifact_digest": target,
        "engine_ref": "terraform",
        "bindings": {"namespace": "x"}
    });
    let (status, _, _) = fixture.send(body, Some(&fixture.etag), None).await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(fixture.revision(2).await.is_err());
    Ok(())
}

#[tokio::test]
async fn absent_target_schema_fails_closed() -> TestResult {
    let fixture = Fixture::new().await?;
    let (target, bytes) = artifact_variants::without_bindings_schema()?;
    let mut upload =
        common::authorized("PUT", &format!("/api/v1/template-artifacts/{target}"), None);
    *upload.body_mut() = axum::body::Body::from(bytes.clone());
    assert_eq!(
        fixture.app.clone().oneshot(upload).await?.status(),
        StatusCode::CREATED
    );
    fixture
        .store
        .store()
        .artifact_archive_put(&target, &bytes, 1_800_000_001_000)
        .await?;
    // Any supplied value would bypass sensitivity and validation.
    let body = serde_json::json!({
        "artifact_digest": target,
        "engine_ref": "terraform",
        "bindings": {"namespace": "x"}
    });
    let (status, _, _) = fixture.send(body, Some(&fixture.etag), None).await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    // An all-keep submission still proceeds (everything retained).
    let body = serde_json::json!({
        "artifact_digest": target,
        "engine_ref": "terraform",
        "bindings": {}
    });
    let (status, detail, _) = fixture.send(body, Some(&fixture.etag), None).await?;
    assert_eq!(status, StatusCode::ACCEPTED, "{detail}");
    let row = fixture.revision(2).await?;
    let stored: serde_json::Value = serde_json::from_str(row.bindings_json.as_deref().unwrap())?;
    assert_eq!(stored["kubeconfig"], serde_json::json!(SECRET));
    Ok(())
}

#[tokio::test]
async fn all_keep_submission_against_same_content_is_a_noop() -> TestResult {
    let (fixture, target) = fixture_with_schema().await?;
    // First update establishes revision 2 under the target artifact.
    let body = serde_json::json!({
        "artifact_digest": target,
        "engine_ref": "terraform",
        "bindings": {"namespace": "renamed-ns"}
    });
    let (status, _, etag) = fixture.send(body, Some(&fixture.etag), None).await?;
    assert_eq!(status, StatusCode::ACCEPTED);
    // Re-asserting the same target with keep-everything bindings is a
    // NoOp: the merged set equals the stored set.
    let body = serde_json::json!({
        "artifact_digest": target,
        "engine_ref": "terraform",
        "bindings": {}
    });
    let (status, _, _) = fixture.send(body, Some(&etag), None).await?;
    assert_eq!(status, StatusCode::OK);
    assert!(fixture.revision(3).await.is_err());
    Ok(())
}

#[tokio::test]
async fn bindings_replay_returns_original_result_and_changed_request_conflicts() -> TestResult {
    let (fixture, target) = fixture_with_schema().await?;
    let body = serde_json::json!({
        "artifact_digest": target,
        "engine_ref": "terraform",
        "bindings": {"namespace": "renamed-ns"},
        "fleet_input_policy": {"size_class": ["standard"]}
    });
    let (first, _, _) = fixture
        .send(body.clone(), Some(&fixture.etag), Some("idem-bindings"))
        .await?;
    assert_eq!(first, StatusCode::ACCEPTED);
    // Exact replay (null sentinel instead of omission is the same merge).
    let replay = serde_json::json!({
        "artifact_digest": target,
        "engine_ref": "terraform",
        "bindings": {"namespace": "renamed-ns", "kubeconfig": null},
        "fleet_input_policy": {"size_class": ["standard"]}
    });
    let (status, _, _) = fixture
        .send(replay, Some(&fixture.etag), Some("idem-bindings"))
        .await?;
    assert_eq!(status, StatusCode::ACCEPTED);
    // A changed request under the same key conflicts.
    let changed = serde_json::json!({
        "artifact_digest": target,
        "engine_ref": "terraform",
        "bindings": {"namespace": "other-ns"},
        "fleet_input_policy": {"size_class": ["standard"]}
    });
    let (status, _, _) = fixture
        .send(changed, Some(&fixture.etag), Some("idem-bindings"))
        .await?;
    assert_eq!(status, StatusCode::CONFLICT);
    Ok(())
}
