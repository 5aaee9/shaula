use axum::http::StatusCode;
use shaula_core::registry::ControlPlaneStore;
use tower::ServiceExt;

use super::common;
use super::support::{body, full_body, json_response, Fixture, TestResult, KEY, PRIVATE_KEY, URI};

#[tokio::test]
async fn accepted_policy_replays_after_activation_and_base_credential_cleanup() -> TestResult {
    let fixture = Fixture::new().await?;
    let original_etag = fixture.etag().await?;
    let request_body = body(1, &["example-org", "another-org"]);
    let first = json_response(
        fixture
            .post(request_body.clone(), &original_etag, "replay")
            .await?,
    )
    .await?;
    fixture.promote(2).await?;
    fixture.execute("UPDATE github_auth_profile_revisions SET credential_bytes=X'' WHERE profile_key='policy-app' AND revision=1").await?;
    let counts = fixture.counts().await?;
    let replay = fixture
        .post(
            body(1, &["another-org", "example-org"]),
            &original_etag,
            "replay",
        )
        .await?;
    assert_eq!(replay.status(), StatusCode::ACCEPTED);
    assert_eq!(json_response(replay).await?, first);
    for (changed_body, changed_etag) in [
        (
            body(2, &["example-org", "another-org"]),
            original_etag.clone(),
        ),
        (body(1, &["example-org"]), original_etag.clone()),
        (request_body, fixture.etag().await?),
    ] {
        let conflict = fixture.post(changed_body, &changed_etag, "replay").await?;
        assert_eq!(conflict.status(), StatusCode::CONFLICT);
        assert_eq!(
            json_response(conflict).await?["code"],
            "IdempotencyConflict"
        );
    }
    assert_eq!(fixture.counts().await?, counts);
    Ok(())
}

#[tokio::test]
async fn policy_and_complete_put_cannot_replay_each_others_idempotency_key() -> TestResult {
    let fixture = Fixture::new().await?;
    let etag = fixture.etag().await?;
    assert_eq!(
        fixture
            .post(body(1, &["example-org"]), &etag, "policy")
            .await?
            .status(),
        StatusCode::ACCEPTED
    );
    let mut request = common::authorized("PUT", URI, Some(full_body(PRIVATE_KEY)));
    request.headers_mut().remove("if-none-match");
    request
        .headers_mut()
        .insert("if-match", fixture.etag().await?.parse()?);
    request
        .headers_mut()
        .insert("idempotency-key", "policy".parse()?);
    let conflict = fixture.app.clone().oneshot(request).await?;
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_response(conflict).await?["code"],
        "IdempotencyConflict"
    );
    fixture.rotate("new-private-key", "rotation").await?;
    let conflict = fixture
        .post(
            body(1, &["example-org"]),
            &fixture.etag().await?,
            "rotation",
        )
        .await?;
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_response(conflict).await?["code"],
        "IdempotencyConflict"
    );
    Ok(())
}

#[tokio::test]
async fn concurrent_identical_policy_requests_commit_one_candidate_and_one_replay() -> TestResult {
    let fixture = Fixture::new().await?;
    let etag = fixture.etag().await?;
    let counts = fixture.counts().await?;
    let (first, second) = tokio::join!(
        fixture.post(
            body(1, &["example-org", "another-org"]),
            &etag,
            "concurrent"
        ),
        fixture.post(
            body(1, &["another-org", "example-org"]),
            &etag,
            "concurrent"
        ),
    );
    let first = first?;
    let second = second?;
    assert_eq!(first.status(), StatusCode::ACCEPTED);
    assert_eq!(second.status(), StatusCode::ACCEPTED);
    assert_eq!(json_response(first).await?, json_response(second).await?);
    assert_eq!(fixture.counts().await?, counts.map(|count| count + 1));
    assert_eq!(
        fixture
            .store
            .auth_profile_get(KEY)
            .await?
            .ok_or("missing head")?
            .desired_revision,
        2
    );
    Ok(())
}

#[tokio::test]
async fn competing_policy_publications_lose_with_a_precondition_error() -> TestResult {
    let fixture = Fixture::new().await?;
    let etag = fixture.etag().await?;
    let counts = fixture.counts().await?;
    let (first, second) = tokio::join!(
        fixture.post(body(1, &["example-org", "first-org"]), &etag, "first"),
        fixture.post(body(1, &["example-org", "second-org"]), &etag, "second"),
    );
    let mut statuses = [first?.status().as_u16(), second?.status().as_u16()];
    statuses.sort();
    assert_eq!(statuses, [202, 412]);
    assert_eq!(fixture.counts().await?, counts.map(|count| count + 1));
    Ok(())
}
