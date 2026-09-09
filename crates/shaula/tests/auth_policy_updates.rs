//! Policy-only HTTP publication through the real daemon and SQLite boundary.

// Existing shared fixtures use unwrap for setup; this suite propagates its own errors.
#[allow(clippy::unwrap_used)]
mod common;
#[path = "auth_policy_updates/guards.rs"]
mod guards;
#[path = "support/unsupported_auth.rs"]
mod history;
#[path = "auth_policy_updates/replay.rs"]
mod replay;
#[path = "auth_policy_updates/support.rs"]
mod support;

use axum::http::StatusCode;
use shaula_core::registry::{AuthPromotionOutcome, ControlPlaneStore};
use support::{body, json_response, Fixture, TestResult, KEY, PRIVATE_KEY, URI};
use tower::ServiceExt;

#[tokio::test]
async fn policy_update_inherits_exact_active_credential_and_stages_normal_validation() -> TestResult
{
    let fixture = Fixture::new().await?;
    let etag = fixture.etag().await?;
    let initial = fixture.counts().await?;
    let response = fixture
        .post(body(1, &["example-org", "another-org"]), &etag, "publish")
        .await?;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let accepted = json_response(response).await?;
    assert_eq!(accepted["revision"], 2);
    assert!(!accepted.to_string().contains(PRIVATE_KEY));
    assert!(!accepted.to_string().contains("private_key"));
    let head = fixture
        .store
        .auth_profile_get(KEY)
        .await?
        .ok_or("missing head")?;
    assert_eq!(head.active_revision, Some(1));
    assert_eq!(head.desired_revision, 2);
    let candidate = fixture
        .store
        .auth_revision_get(KEY, 2)
        .await?
        .ok_or("missing candidate")?;
    assert_eq!(candidate.kind, "github_app");
    assert_eq!(candidate.app_id.as_deref(), Some("4863460"));
    assert_eq!(candidate.state, "Validating");
    assert_eq!(
        candidate
            .target_policy()?
            .ok_or("missing policy")?
            .selectors()
            .len(),
        2
    );
    assert!(candidate.validation_snapshot_json.is_none());
    assert!(fixture.store.auth_bindings_get(KEY, 2).await?.is_empty());
    assert_eq!(
        fixture.store.auth_credential_bytes(KEY, 2).await?,
        Some(PRIVATE_KEY.as_bytes().to_vec())
    );
    assert_eq!(
        fixture.store.auth_credential_bytes(KEY, 1).await?,
        Some(PRIVATE_KEY.as_bytes().to_vec())
    );
    assert_eq!(fixture.counts().await?, initial.map(|count| count + 1));
    assert_eq!(fixture.promote(2).await?, AuthPromotionOutcome::Promoted);
    assert_eq!(
        fixture
            .store
            .auth_profile_get(KEY)
            .await?
            .ok_or("missing head")?
            .active_revision,
        Some(2)
    );
    Ok(())
}

#[tokio::test]
async fn identical_policy_is_an_explicit_revalidation_candidate() -> TestResult {
    let fixture = Fixture::new().await?;
    let response = fixture
        .post(
            body(1, &["example-org"]),
            &fixture.etag().await?,
            "revalidate",
        )
        .await?;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(json_response(response).await?["revision"], 2);
    assert_eq!(
        fixture
            .store
            .auth_profile_get(KEY)
            .await?
            .ok_or("missing head")?
            .active_revision,
        Some(1)
    );
    Ok(())
}

#[tokio::test]
async fn policy_update_rejects_invalid_or_credential_bearing_payloads_without_writes() -> TestResult
{
    let fixture = Fixture::new().await?;
    let etag = fixture.etag().await?;
    let initial = fixture.counts().await?;
    let valid = body(1, &["example-org"]);
    let mut bodies = vec![
        body(0, &["example-org"]),
        body(-1, &["example-org"]),
        body(1, &[]),
        body(1, &["../escape"]),
        r#"{"base_revision":1,"base_revision":2,"target_policy":[]}"#.into(),
        r#"{"base_revision":1.5,"target_policy":[]}"#.into(),
        r#"{"base_revision":9223372036854775808,"target_policy":[]}"#.into(),
        r#"{"target_policy":[]}"#.into(),
    ];
    for field in ["private_key", "app_id", "installation_id", "unknown"] {
        let mut value: serde_json::Value = serde_json::from_str(&valid)?;
        value[field] = "must-not-be-accepted".into();
        bodies.push(value.to_string());
    }
    for body in bodies {
        let response = fixture.post(body, &etag, "invalid").await?;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!json_response(response)
            .await?
            .to_string()
            .contains("must-not-be-accepted"));
    }
    assert_eq!(fixture.counts().await?, initial);
    Ok(())
}

#[tokio::test]
async fn policy_update_requires_both_scopes_and_if_match() -> TestResult {
    let fixture = Fixture::new().await?;
    let initial = fixture.counts().await?;
    for scopes in ["auth.read", "auth.write", "fleet.read"] {
        let mut request = common::authorized(
            "POST",
            &format!("{URI}/policy-updates"),
            Some(body(1, &["example-org"])),
        );
        request
            .headers_mut()
            .insert("authorization", common::oidc::bearer(scopes).parse()?);
        request
            .headers_mut()
            .insert("if-match", fixture.etag().await?.parse()?);
        assert_eq!(
            fixture.app.clone().oneshot(request).await?.status(),
            StatusCode::FORBIDDEN
        );
    }
    let request = common::authorized(
        "POST",
        &format!("{URI}/policy-updates"),
        Some(body(1, &["example-org"])),
    );
    assert_eq!(
        fixture.app.clone().oneshot(request).await?.status(),
        StatusCode::PRECONDITION_REQUIRED
    );
    assert_eq!(fixture.counts().await?, initial);
    Ok(())
}

#[tokio::test]
async fn policy_update_failure_rolls_back_candidate_credential_and_all_publication_facts(
) -> TestResult {
    let fixture = Fixture::new().await?;
    let etag = fixture.etag().await?;
    let initial = fixture.counts().await?;
    fixture.execute("CREATE TRIGGER reject_policy_outbox BEFORE INSERT ON outbox BEGIN SELECT RAISE(FAIL, 'protected-fixture-marker'); END").await?;
    let response = fixture
        .post(body(1, &["another-org"]), &etag, "atomic")
        .await?;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!json_response(response)
        .await?
        .to_string()
        .contains("protected-fixture-marker"));
    assert_eq!(fixture.counts().await?, initial);
    assert_eq!(fixture.etag().await?, etag);
    assert!(fixture.store.auth_credential_bytes(KEY, 2).await?.is_none());
    fixture.execute("DROP TRIGGER reject_policy_outbox").await?;
    assert_eq!(
        fixture
            .post(body(1, &["another-org"]), &etag, "atomic")
            .await?
            .status(),
        StatusCode::ACCEPTED
    );
    Ok(())
}

#[tokio::test]
async fn policy_updates_reject_create_only_preconditions_even_with_if_match() -> TestResult {
    let fixture = Fixture::new().await?;
    let counts = fixture.counts().await?;
    for with_match in [false, true] {
        let mut request = common::authorized(
            "POST",
            &format!("{URI}/policy-updates"),
            Some(body(1, &["example-org"])),
        );
        request.headers_mut().insert("if-none-match", "*".parse()?);
        if with_match {
            request
                .headers_mut()
                .insert("if-match", fixture.etag().await?.parse()?);
        }
        assert_eq!(
            fixture.app.clone().oneshot(request).await?.status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(fixture.counts().await?, counts);
    Ok(())
}
