//! The new HTTP policy command enters the existing v2 validation pipeline.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use shaula_core::registry::{ControlPlaneStore, ProfileRegistryPort};
use std::sync::Arc;
use tower::ServiceExt;

use super::fixture::{oidc, Fixture, TestResult, KEY};

#[tokio::test]
async fn auth_policy_update_runs_existing_validator_before_activation() -> TestResult {
    let test = Fixture::new(true).await?;
    test.seed(true).await?;
    let original_credential = test
        .plane
        .control_plane
        .auth_credential_bytes(KEY, 1)
        .await?;
    let response = test.app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri(format!("/api/v1/github-auth-profiles/{KEY}/policy-updates"))
            .header("authorization", oidc::bearer(oidc::SCOPES))
            .header("content-type", "application/json")
            .header("if-match", "\"inc-v2:1\"")
            .header("idempotency-key", "policy-validation-composition")
            .body(Body::from(r#"{"base_revision":1,"target_policy":[{"kind":"organization","owner":"Indexyz"}]}"#))?,
    ).await?;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let store = &test.plane.control_plane;
    let head = store.auth_profile_get(KEY).await?.ok_or("head missing")?;
    assert_eq!((head.desired_revision, head.active_revision), (2, Some(1)));
    let candidate = store
        .auth_revision_get(KEY, 2)
        .await?
        .ok_or("candidate missing")?;
    assert_eq!(candidate.state, "Validating");
    assert!(original_credential == store.auth_credential_bytes(KEY, 2).await?);
    assert_eq!(
        test.calls(),
        0,
        "publication does not resolve a GitHub installation link"
    );

    let github = crate::auth_worker_mock::mock_server(false).await;
    let control: Arc<dyn ControlPlaneStore> = store.clone();
    let clock: Arc<dyn shaula_core::ports::Clock> = Arc::new(crate::auth_worker_mock::Now);
    let outcome = crate::auth_worker_v2::validate_v2(
        &control,
        &clock,
        KEY,
        &candidate,
        &crate::auth_worker_mock::endpoints(&github.base),
    )
    .await?;
    assert_eq!(outcome, crate::auth_worker_v2::Verdict::Accepted);
    let head = store.auth_profile_get(KEY).await?.ok_or("head missing")?;
    assert_eq!(
        (
            head.desired_revision,
            head.active_revision,
            head.status.as_str()
        ),
        (2, Some(2), "Active")
    );
    let bindings = store.auth_bindings_get(KEY, 2).await?;
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].login, "Indexyz");
    assert!(original_credential == store.auth_credential_bytes(KEY, 1).await?);

    // The regular read model now attributes policy and bindings to r2.
    let plane = shaula_daemon::service::ControlPlane::new(
        control,
        clock,
        b"policy-validation".to_vec(),
        100,
        "unused-engine".into(),
    );
    let actor = shaula_core::registry::Actor {
        name: "operator".into(),
        scopes: vec![shaula_core::registry::Scope::AuthRead],
    };
    let view = plane
        .auth_get(&actor, KEY)
        .await?
        .map_err(|_| "profile read failed")?;
    assert_eq!(view.active_revision, Some(2));
    Ok(())
}
