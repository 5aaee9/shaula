use axum::http::StatusCode;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use shaula_core::registry::{Actor, MutationError};
use tower::ServiceExt;

use super::{common, registry::*, support::*};

// Frozen independently from the pre-C2 formats, not calculated with production
// canonicalization/hash helpers. Each part is length-prefixed: "len:bytes|".
// Fleet/Pool include "if-match:historic:7"; Template/Auth omit conditions.
// Template canonical body uses the now-unavailable digest below, no bindings.
// GitHub: 2|4863460|{"selectors":[{"kind":"organization","owner":"example-org"}]}
// Forgejo: forgejo_token|1|{"instance_url":"https://forgejo.example.test","scope":{"kind":"instance"}}
const GOLDENS: [(Kind, &str); 5] = [
    (
        Kind::Fleet,
        "sha256:09c520697b21d4ec7f8ac86f98b71cd2811bc1243b8712a081d24eb7677269d5",
    ),
    (
        Kind::Pool,
        "sha256:96e90dde7ef9740cbccfc21c981a1dd4f61a07d3661d82c24e55d2997c8c7fee",
    ),
    (
        Kind::Template,
        "sha256:39109dad6b6374c65017ab90d0a1cf95ac5ad5ce747b9008530ca234e5780c26",
    ),
    (
        Kind::Github,
        "sha256:d3869b0bc9838d5715bf206d3b4514278bf82f79249ccf8b67c1244bb18802a3",
    ),
    (
        Kind::Forgejo,
        "sha256:53890eacc782378b47d5bde644c6871b96d7fb80ceabac29780dcc1012b72c2b",
    ),
];

#[tokio::test]
async fn unowned_frozen_history_is_rejected_before_mixed_conditions() -> TestResult {
    let fixture = Fixture::new().await?;
    let unavailable = format!("sha256:{}", "a".repeat(64));
    for (kind, hash) in GOLDENS {
        let original = success(
            kind.publish(
                &fixture.service,
                &actor(),
                &fixture.digest,
                0,
                Conditions::create(),
            )
            .await?,
        )?;
        // Advance the head and rotate protected material. Replay must compare the
        // recorded revision's bytes, not today's credential/bindings or authority.
        success(
            kind.publish(
                &fixture.service,
                &actor(),
                &fixture.digest,
                1,
                Conditions::replace(&original, "advance")?,
            )
            .await?,
        )?;
        let mut body = serde_json::to_value(&original)?;
        body.as_object_mut()
            .ok_or("accepted object")?
            .remove("no_op"); // oldest persisted shape
        fixture.db.execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "INSERT INTO idempotency_records (id,resource_kind,resource_key,idempotency_key,request_hash,response_status,response_body,created_at) VALUES (?,?,?,?,?,202,?,0)",
            vec![kind.key().into(), kind.resource_kind().into(), kind.key().into(), "legacy-fixed".into(), hash.into(), body.to_string().into()],
        )).await?;
        let digest = if matches!(kind, Kind::Template) {
            &unavailable
        } else {
            &fixture.digest
        };
        // Fleet/Pool previously admitted mixed-condition creates; Profile hashes
        // did not include conditions. Their stored responses remain reachable
        // even though a NEW mixed request is now 400.
        let mut request = common::put_with_idempotency(
            &kind.uri(),
            "legacy-fixed",
            kind.body(digest, 0)?.to_string(),
        );
        request
            .headers_mut()
            .insert("if-match", "\"historic:7\"".parse()?);
        let response = fixture.app.clone().oneshot(request).await?;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await?;
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes)?["code"],
            "LegacyIdempotencyConflict"
        );
        assert_eq!(kind.revision(&fixture).await?, 2);

        let denied = kind
            .publish(
                &fixture.service,
                &Actor {
                    authentication: Default::default(),
                    name: "no-grants".into(),
                    scopes: vec![],
                },
                digest,
                0,
                Conditions {
                    create: true,
                    expected: Some(("historic".into(), 7)),
                    idem: Some("legacy-fixed".into()),
                },
            )
            .await?;
        assert!(
            matches!(denied, Err(MutationError::Unprocessable { .. })),
            "{kind:?}: {denied:?}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn publication_metadata_never_contains_protected_material() -> TestResult {
    let fixture = Fixture::new().await?;
    for kind in [Kind::Template, Kind::Github, Kind::Forgejo] {
        let mut create = Conditions::create();
        create.idem = Some("publish".into());
        let base = success(
            kind.publish(&fixture.service, &actor(), &fixture.digest, 0, create)
                .await?,
        )?;
        success(
            kind.publish(
                &fixture.service,
                &actor(),
                &fixture.digest,
                1,
                Conditions::replace(&base, "rotate")?,
            )
            .await?,
        )?;
    }
    let rows = fixture.db.query_all(Statement::from_string(DatabaseBackend::Sqlite,
        "SELECT COALESCE(response_body, '') AS metadata FROM idempotency_records UNION ALL SELECT COALESCE(detail_json, '') AS metadata FROM audit_records UNION ALL SELECT payload AS metadata FROM outbox",
    )).await?;
    assert!(!rows.is_empty());
    for row in rows {
        let metadata: String = row.try_get("", "metadata")?;
        for secret in [
            "secret-kubeconfig",
            "github_app_test_key_bytes",
            "private-forgejo-token",
            "rotated-kubeconfig",
            "rotated-private-key",
            "rotated-token",
        ] {
            assert!(
                !metadata.contains(secret),
                "secret appeared in public mutation metadata"
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn identical_auth_put_still_creates_a_candidate_without_a_replay_key() -> TestResult {
    let fixture = Fixture::new().await?;
    for kind in [Kind::Github, Kind::Forgejo] {
        let original = success(
            kind.publish(
                &fixture.service,
                &actor(),
                &fixture.digest,
                0,
                Conditions::create(),
            )
            .await?,
        )?;
        let mut conditions = Conditions::replace(&original, "unused")?;
        conditions.idem = None;
        let candidate = success(
            kind.publish(&fixture.service, &actor(), &fixture.digest, 0, conditions)
                .await?,
        )?;
        assert!(!candidate.no_op);
        assert_eq!(candidate.change.revision, 2);
        assert_ne!(candidate.change.id, original.change.id);
    }
    Ok(())
}
