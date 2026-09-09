use crate::common;
#[path = "template_update_concurrency.rs"]
mod concurrency;
use crate::template_updates as support;

use axum::body::Body;
use axum::http::{HeaderValue, StatusCode};
use shaula_core::registry::ControlPlaneStore;
use tower::ServiceExt;

use support::{request, response, Fixture, TestResult, PROFILE};

#[tokio::test]
async fn update_inherits_exact_configuration_and_preserves_history_and_fleet_pin() -> TestResult {
    let fixture = Fixture::new().await?;
    let base = fixture.revision(1).await?;
    let protected = fixture
        .store
        .template_protected_bindings("k8s-linux", 1)
        .await?;
    let fleet_before = fixture.store.fleet_revision_latest("build").await?;
    let (status, body, _) = fixture
        .send(fixture.body(), Some(&fixture.etag), Some("update"))
        .await?;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert!(!body.contains("secret-kubeconfig"));
    let revision = fixture.revision(2).await?;
    assert_eq!(revision.artifact_digest, fixture.target_digest);
    assert_eq!(
        revision.fleet_input_policy_json,
        base.fleet_input_policy_json
    );
    let inherited = fixture
        .store
        .template_protected_bindings("k8s-linux", 2)
        .await?;
    assert_eq!(
        inherited.as_ref().map(|pair| &pair.0),
        protected.as_ref().map(|pair| &pair.0)
    );
    let before_scan = common::get_json(&fixture.app, PROFILE).await;
    assert_eq!(before_scan["desiredRevision"], 2);
    assert_eq!(before_scan["activeRevision"], 1);
    fixture.store.periodic_scan(1_800_000_002_000).await?;
    assert_eq!(
        common::get_json(&fixture.app, PROFILE).await["activeRevision"],
        2
    );
    let old = fixture.revision(1).await?;
    assert_eq!(old.artifact_digest, fixture.base_digest);
    assert_eq!(old.fleet_input_policy_json, base.fleet_input_policy_json);
    assert_eq!(
        fixture
            .store
            .template_protected_bindings("k8s-linux", 1)
            .await?,
        protected
    );
    let fleet_after = fixture.store.fleet_revision_latest("build").await?;
    assert_eq!(
        fleet_after
            .as_ref()
            .map(|row| (&row.spec_json, row.template_revision)),
        fleet_before
            .as_ref()
            .map(|row| (&row.spec_json, row.template_revision))
    );
    for uri in [
        PROFILE.to_string(),
        format!("{PROFILE}/revisions/2"),
        format!("{PROFILE}/revisions/2/input-contract"),
    ] {
        let text = common::get_json(&fixture.app, &uri).await.to_string();
        assert!(
            !text.contains("secret-kubeconfig"),
            "secret leaked through {uri}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn update_replaces_policy_only_when_explicitly_requested() -> TestResult {
    let fixture = Fixture::new().await?;
    let mut body = fixture.body();
    let policy = serde_json::json!({"size_class":["large"], "limit":[18446744073709551615u64]});
    body["fleet_input_policy"] = policy.clone();
    let (status, text, _) = fixture.send(body, Some(&fixture.etag), None).await?;
    assert_eq!(status, StatusCode::ACCEPTED, "{text}");
    let stored: serde_json::Value = serde_json::from_str(
        fixture
            .revision(2)
            .await?
            .fleet_input_policy_json
            .as_deref()
            .ok_or("policy missing")?,
    )?;
    assert_eq!(stored, policy);
    assert_ne!(
        fixture.revision(1).await?.fleet_input_policy_json,
        Some(policy.to_string())
    );
    Ok(())
}

#[tokio::test]
async fn update_replay_keeps_original_base_after_head_and_secret_changes() -> TestResult {
    let fixture = Fixture::new().await?;
    let first = fixture
        .send(fixture.body(), Some(&fixture.etag), Some("update"))
        .await?;
    assert_eq!(first.0, StatusCode::ACCEPTED);
    let mut third: serde_json::Value = serde_json::from_str(
        &common::TEMPLATE_PUT_BODY.replace("PLACEHOLDER", &fixture.target_digest),
    )?;
    third["bindings"]["kubeconfig"] = "rotated-secret".into();
    third["fleet_input_policy"] = serde_json::json!({"size_class":["different"]});
    let mut put = common::authorized("PUT", PROFILE, Some(third.to_string()));
    put.headers_mut().remove("if-none-match");
    put.headers_mut()
        .insert("if-match", HeaderValue::from_str(&first.2)?);
    assert_eq!(
        fixture.app.clone().oneshot(put).await?.status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(
        fixture
            .send(fixture.body(), Some(&fixture.etag), Some("update"))
            .await?,
        first
    );
    assert_eq!(
        common::get_json(&fixture.app, PROFILE).await["desiredRevision"],
        3
    );
    assert_eq!(
        fixture
            .send(fixture.body(), Some(&fixture.etag), Some("fresh"))
            .await?
            .0,
        StatusCode::PRECONDITION_FAILED
    );
    // Changing only the base must not accidentally replay the old response.
    assert_eq!(
        fixture
            .send(fixture.body(), Some(&first.2), Some("update"))
            .await?
            .0,
        StatusCode::CONFLICT
    );
    let mut changed = fixture.body();
    changed["engine_ref"] = "another-engine".into();
    assert_eq!(
        fixture
            .send(changed, Some(&fixture.etag), Some("update"))
            .await?
            .0,
        StatusCode::CONFLICT
    );
    let mut explicit_same = fixture.body();
    explicit_same["fleet_input_policy"] = serde_json::json!({"size_class":["standard"]});
    assert_eq!(
        fixture
            .send(explicit_same, Some(&fixture.etag), Some("update"))
            .await?
            .0,
        StatusCode::CONFLICT
    );
    Ok(())
}

#[tokio::test]
async fn update_authorizes_and_rejects_strict_payload_and_invalid_preconditions() -> TestResult {
    let fixture = Fixture::new().await?;
    assert_eq!(
        fixture.send(fixture.body(), None, None).await?.0,
        StatusCode::PRECONDITION_REQUIRED
    );
    assert_eq!(
        fixture
            .send(fixture.body(), Some("\"wrong-incarnation:1\""), None)
            .await?
            .0,
        StatusCode::PRECONDITION_FAILED
    );
    let absent = fixture.etag.replace(":1\"", ":99\"");
    assert_eq!(
        fixture.send(fixture.body(), Some(&absent), None).await?.0,
        StatusCode::NOT_FOUND
    );
    for invalid in [
        serde_json::json!({"bindings":{"kubeconfig":"must-not-be-accepted"}}),
        serde_json::json!({"fleet_input_policy":null}),
        serde_json::json!({"fleet_input_policy":[]}),
        serde_json::json!({"fleet_input_policy":"x"}),
        serde_json::json!({"unknown":true}),
    ] {
        let mut body = fixture.body();
        body.as_object_mut()
            .ok_or("body object missing")?
            .extend(invalid.as_object().ok_or("invalid object missing")?.clone());
        let result = fixture.send(body, Some(&fixture.etag), None).await?;
        assert_eq!(result.0, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!result.1.contains("must-not-be-accepted"));
    }
    let mut denied = request(fixture.body(), Some(&fixture.etag), Some("key"))?;
    denied.headers_mut().insert(
        "authorization",
        HeaderValue::from_str(&common::oidc::bearer("template.read"))?,
    );
    assert_eq!(
        fixture.app.clone().oneshot(denied).await?.status(),
        StatusCode::FORBIDDEN
    );
    let mut create = request(fixture.body(), Some(&fixture.etag), None)?;
    create
        .headers_mut()
        .insert("if-none-match", HeaderValue::from_static("*"));
    assert_eq!(
        fixture.app.clone().oneshot(create).await?.status(),
        StatusCode::BAD_REQUEST
    );
    let mut missing = request(fixture.body(), Some(&fixture.etag), None)?;
    *missing.uri_mut() = "/api/v1/template-profiles/missing/updates".parse()?;
    assert_eq!(
        fixture.app.clone().oneshot(missing).await?.status(),
        StatusCode::NOT_FOUND
    );
    assert!(fixture
        .store
        .template_revision_get("k8s-linux", 2)
        .await?
        .is_none());
    Ok(())
}

#[tokio::test]
async fn concurrent_updates_admit_only_one_candidate_from_the_same_base() -> TestResult {
    let fixture = Fixture::new().await?;
    let (left, right) = tokio::join!(
        fixture.send(fixture.body(), Some(&fixture.etag), Some("left")),
        fixture.send(fixture.body(), Some(&fixture.etag), Some("right")),
    );
    let mut statuses = [left?.0.as_u16(), right?.0.as_u16()];
    statuses.sort_unstable();
    assert_eq!(statuses, [202, 412]);
    assert!(fixture
        .store
        .template_revision_get("k8s-linux", 3)
        .await?
        .is_none());
    Ok(())
}

#[tokio::test]
async fn update_noop_replays_and_cannot_alias_an_ordinary_put() -> TestResult {
    let fixture = Fixture::new().await?;
    let mut body = fixture.body();
    body["artifact_digest"] = fixture.base_digest.clone().into();
    let first = fixture
        .send(body.clone(), Some(&fixture.etag), Some("noop"))
        .await?;
    assert_eq!(first.0, StatusCode::OK);
    assert_eq!(
        fixture
            .send(body, Some(&fixture.etag), Some("noop"))
            .await?,
        first
    );
    let mut put = common::authorized(
        "PUT",
        PROFILE,
        Some(common::TEMPLATE_PUT_BODY.replace("PLACEHOLDER", &fixture.base_digest)),
    );
    put.headers_mut().remove("if-none-match");
    put.headers_mut()
        .insert("if-match", HeaderValue::from_str(&fixture.etag)?);
    put.headers_mut()
        .insert("idempotency-key", HeaderValue::from_static("noop"));
    assert_eq!(
        response(fixture.app.clone().oneshot(put).await?).await?.0,
        StatusCode::CONFLICT
    );
    assert!(fixture
        .store
        .template_revision_get("k8s-linux", 2)
        .await?
        .is_none());
    Ok(())
}

#[tokio::test]
async fn update_keeps_admission_compatibility_and_retirement_guards() -> TestResult {
    let fixture = Fixture::new().await?;
    let (digest, bytes) = common::fixture_artifact_docker();
    let mut upload =
        common::authorized("PUT", &format!("/api/v1/template-artifacts/{digest}"), None);
    *upload.body_mut() = Body::from(bytes);
    assert_eq!(
        fixture.app.clone().oneshot(upload).await?.status(),
        StatusCode::CREATED
    );
    let mut incompatible = fixture.body();
    incompatible["artifact_digest"] = digest.into();
    assert_eq!(
        fixture
            .send(incompatible, Some(&fixture.etag), None)
            .await?
            .0,
        StatusCode::CONFLICT
    );
    let mut retire = common::authorized("DELETE", PROFILE, None);
    retire
        .headers_mut()
        .insert("if-match", HeaderValue::from_str(&fixture.etag)?);
    assert_eq!(
        fixture.app.clone().oneshot(retire).await?.status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(
        fixture
            .send(fixture.body(), Some(&fixture.etag), None)
            .await?
            .0,
        StatusCode::CONFLICT
    );
    Ok(())
}
