use axum::http::StatusCode;
use sea_orm::{ConnectionTrait, TransactionTrait};
use shaula_core::registry::ControlPlaneStore;
use tower::ServiceExt;

use super::support::{body, json_response, Fixture, TestResult, KEY, PRIVATE_KEY};

#[tokio::test]
async fn policy_update_uses_active_credential_while_a_different_candidate_is_desired() -> TestResult
{
    let fixture = Fixture::new().await?;
    fixture.rotate("candidate-private-key", "rotation").await?;
    let response = fixture
        .post(
            body(1, &["example-org", "another-org"]),
            &fixture.etag().await?,
            "update-active",
        )
        .await?;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(json_response(response).await?["revision"], 3);
    assert_eq!(
        fixture.store.auth_credential_bytes(KEY, 3).await?,
        Some(PRIVATE_KEY.as_bytes().to_vec())
    );
    assert_eq!(
        fixture.store.auth_credential_bytes(KEY, 2).await?,
        Some(b"candidate-private-key".to_vec())
    );
    Ok(())
}

#[tokio::test]
async fn activation_with_unchanged_desired_etag_invalidates_reviewed_base() -> TestResult {
    let fixture = Fixture::new().await?;
    fixture.rotate("candidate-private-key", "rotation").await?;
    let etag = fixture.etag().await?;
    let counts = fixture.counts().await?;
    let transaction = fixture.db.begin().await?;
    transaction.execute_unprepared("UPDATE github_auth_profiles SET active_revision=2, status='Active' WHERE key='policy-app'").await?;
    transaction.execute_unprepared("UPDATE github_auth_profile_revisions SET state='Active' WHERE profile_key='policy-app' AND revision=2").await?;
    let pending = fixture.post(body(1, &["example-org"]), &etag, "racing-activation");
    tokio::pin!(pending);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut pending)
            .await
            .is_err(),
        "publication waits for the writer before reading its Active base"
    );
    transaction.commit().await?;
    let response = pending.await?;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(json_response(response).await?["detail"]
        .as_str()
        .is_some_and(|detail| detail.contains("Active base")));
    assert_eq!(fixture.etag().await?, etag);
    assert_eq!(fixture.counts().await?, counts);
    let response = fixture
        .post(body(2, &["example-org"]), &etag, "reviewed-new-base")
        .await?;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(
        fixture.store.auth_credential_bytes(KEY, 3).await?,
        Some(b"candidate-private-key".to_vec())
    );
    Ok(())
}

#[tokio::test]
async fn unavailable_base_or_retirement_never_creates_empty_or_fallback_credentials() -> TestResult
{
    for sql in [
        "UPDATE github_auth_profiles SET status='Unsupported' WHERE key='policy-app'",
        "UPDATE github_auth_profiles SET status='UnknownFutureState' WHERE key='policy-app'",
        "UPDATE github_auth_profiles SET active_revision=NULL WHERE key='policy-app'",
        "UPDATE github_auth_profiles SET deletion_requested=1, status='Retiring' WHERE key='policy-app'",
        "UPDATE github_auth_profile_revisions SET credential_bytes=X'' WHERE profile_key='policy-app' AND revision=1",
        "UPDATE github_auth_profile_revisions SET schema_version=1 WHERE profile_key='policy-app' AND revision=1",
        "UPDATE github_auth_profile_revisions SET kind='pat' WHERE profile_key='policy-app' AND revision=1",
    ] {
        let fixture = Fixture::new().await?;
        let etag = fixture.etag().await?;
        let counts = fixture.counts().await?;
        fixture.execute(sql).await?;
        let response = fixture.post(body(1, &["example-org"]), &etag, "blocked").await?;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(fixture.counts().await?, counts);
        assert!(fixture.store.auth_credential_bytes(KEY, 2).await?.is_none());
    }
    Ok(())
}

#[tokio::test]
async fn historical_profiles_cannot_supply_policy_update_credentials() -> TestResult {
    let fixture = Fixture::new().await?;
    super::history::seed(&fixture.engine, "historical-app", "github_app").await?;
    let counts = fixture.counts().await?;
    let mut request = super::common::authorized(
        "POST",
        "/api/v1/github-auth-profiles/historical-app/policy-updates",
        Some(body(1, &["example-org"])),
    );
    request
        .headers_mut()
        .insert("if-match", "\"historical-app-inc:1\"".parse()?);
    assert_eq!(
        fixture.app.clone().oneshot(request).await?.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(fixture.counts().await?, counts);
    Ok(())
}

#[tokio::test]
async fn profile_identity_and_desired_head_are_both_fenced() -> TestResult {
    let fixture = Fixture::new().await?;
    let old_etag = fixture.etag().await?;
    fixture.rotate("new-private-key", "rotation").await?;
    assert_eq!(
        fixture
            .post(body(1, &["example-org"]), &old_etag, "old-head")
            .await?
            .status(),
        StatusCode::PRECONDITION_FAILED
    );
    let etag = fixture.etag().await?;
    let counts = fixture.counts().await?;
    fixture.execute("UPDATE github_auth_profiles SET incarnation='recreated-profile' WHERE key='policy-app'").await?;
    assert_eq!(
        fixture
            .post(body(1, &["example-org"]), &etag, "old-incarnation")
            .await?
            .status(),
        StatusCode::PRECONDITION_FAILED
    );
    assert_eq!(fixture.counts().await?, counts);
    fixture
        .execute("DELETE FROM github_auth_profiles WHERE key='policy-app'")
        .await?;
    assert_eq!(
        fixture
            .post(body(1, &["example-org"]), &etag, "missing-profile")
            .await?
            .status(),
        StatusCode::NOT_FOUND
    );
    Ok(())
}
