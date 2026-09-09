//! Composition checks use the real protected store read and HTTP/OIDC route.

use super::*;
use axum::http::StatusCode;

#[path = "auth_installation_link_fixture.rs"]
mod fixture;
use fixture::{Fixture, TestResult, KEY, URI};

#[path = "auth_policy_validation_tests.rs"]
mod policy_validation;

#[tokio::test]
async fn installation_link_uses_active_identity_and_preserves_durable_state() -> TestResult {
    let test = Fixture::new(true).await?;
    test.seed(true).await?;
    crate::auth_worker_v2::tests::seed_candidate(
        &test.plane.control_plane,
        2,
        &crate::auth_worker_v2::tests::policy(true),
    )
    .await;
    // An invalid pending credential must never replace the usable Active key.
    test.sql(
        "UPDATE github_auth_profile_revisions SET credential_bytes=x'626164' WHERE revision=2",
    )
    .await?;
    let counts = test.table_counts().await?;
    let before = test
        .plane
        .control_plane
        .auth_revision_get(KEY, 1)
        .await?
        .ok_or("active missing")?;
    let credential = test
        .plane
        .control_plane
        .auth_credential_bytes(KEY, 1)
        .await?;
    let bindings = test.plane.control_plane.auth_bindings_get(KEY, 1).await?;
    let (status, body) = test.get(URI, Some(fixture::oidc::SCOPES)).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        serde_json::json!({
            "url":"https://github.com/apps/shaula-fixture/installations/new",
            "appId":"4863460", "revision":1, "incarnation":"inc-v2",
        })
    );
    assert_eq!(test.calls(), 1);
    assert_eq!(counts, test.table_counts().await?);
    let head = test
        .plane
        .control_plane
        .auth_profile_get(KEY)
        .await?
        .ok_or("head missing")?;
    assert_eq!(
        (
            head.active_revision,
            head.desired_revision,
            head.status.as_str()
        ),
        (Some(1), 2, "Validating")
    );
    let after = test
        .plane
        .control_plane
        .auth_revision_get(KEY, 1)
        .await?
        .ok_or("active missing")?;
    assert_eq!(before.policy_json, after.policy_json);
    assert_eq!(
        bindings,
        test.plane.control_plane.auth_bindings_get(KEY, 1).await?
    );
    assert!(
        credential
            == test
                .plane
                .control_plane
                .auth_credential_bytes(KEY, 1)
                .await?
    );
    Ok(())
}

#[tokio::test]
async fn installation_link_checks_both_scopes_and_missing_profiles_before_github() -> TestResult {
    let test = Fixture::new(true).await?;
    for scopes in ["auth.read", "auth.write", "fleet.read"] {
        let (status, body) = test.get(URI, Some(scopes)).await?;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["code"], "ScopeDenied");
    }
    assert_eq!(test.get(URI, None).await?.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        test.get(URI, Some(fixture::oidc::SCOPES)).await?.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(test.calls(), 0);
    let actor = Actor {
        name: "reader".into(),
        scopes: vec![Scope::AuthRead],
    };
    assert_eq!(
        test.links.installation_link(&actor, KEY).await,
        Err(Error::ScopeDenied)
    );
    Ok(())
}

#[tokio::test]
async fn installation_link_refuses_unavailable_and_historical_authority() -> TestResult {
    let test = Fixture::new(true).await?;
    test.seed(false).await?;
    assert_eq!(
        test.get(URI, Some(fixture::oidc::SCOPES)).await?.0,
        StatusCode::CONFLICT
    );
    test.sql("UPDATE github_auth_profiles SET active_revision=1, status='Active'; UPDATE github_auth_profile_revisions SET state='Active'").await?;
    for sql in [
        "UPDATE github_auth_profiles SET status='Retiring'",
        "UPDATE github_auth_profiles SET status='Retired'",
        "UPDATE github_auth_profiles SET status='Unsupported'",
        "UPDATE github_auth_profiles SET status='Active'; UPDATE github_auth_profile_revisions SET schema_version=1",
        "UPDATE github_auth_profile_revisions SET schema_version=2, kind='pat'",
    ] {
        test.sql(sql).await?;
        let (status, body) = test.get(URI, Some(fixture::oidc::SCOPES)).await?;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["code"], "AuthProfileUnavailable");
    }
    assert_eq!(test.calls(), 0);
    Ok(())
}

#[tokio::test]
async fn installation_link_errors_are_sanitized_and_retryable_without_health_mutations(
) -> TestResult {
    let test = Fixture::new(true).await?;
    test.seed(true).await?;
    for (remote, body, expected, code) in [
        (
            StatusCode::FOUND,
            "PRIVATE KEY provider redirect",
            StatusCode::BAD_GATEWAY,
            "GitHubAppInvalid",
        ),
        (
            StatusCode::UNAUTHORIZED,
            "PRIVATE KEY provider secret",
            StatusCode::BAD_GATEWAY,
            "GitHubAppInvalid",
        ),
        (
            StatusCode::FORBIDDEN,
            "PRIVATE KEY provider secret",
            StatusCode::BAD_GATEWAY,
            "GitHubAppInvalid",
        ),
        (
            StatusCode::TOO_MANY_REQUESTS,
            "PRIVATE KEY provider secret",
            StatusCode::SERVICE_UNAVAILABLE,
            "GitHubAppUnavailable",
        ),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "PRIVATE KEY provider secret",
            StatusCode::SERVICE_UNAVAILABLE,
            "GitHubAppUnavailable",
        ),
        (
            StatusCode::OK,
            "PRIVATE KEY invalid JSON",
            StatusCode::BAD_GATEWAY,
            "GitHubAppInvalid",
        ),
        (
            StatusCode::OK,
            r#"{"id":123,"slug":"shaula"}"#,
            StatusCode::BAD_GATEWAY,
            "GitHubAppInvalid",
        ),
        (
            StatusCode::OK,
            r#"{"id":4863460,"slug":"../evil"}"#,
            StatusCode::BAD_GATEWAY,
            "GitHubAppInvalid",
        ),
    ] {
        test.reply(remote, body).await;
        let (status, body) = test.get(URI, Some(fixture::oidc::SCOPES)).await?;
        assert_eq!(status, expected);
        assert_eq!(body["code"], code);
    }
    assert_eq!(
        test.plane
            .control_plane
            .auth_profile_get(KEY)
            .await?
            .ok_or("head missing")?
            .status,
        "Active"
    );
    let unwired = Fixture::new(false).await?;
    assert_eq!(
        unwired.get(URI, Some(fixture::oidc::SCOPES)).await?.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    Ok(())
}

#[tokio::test]
async fn installation_link_rechecks_identity_after_remote_read() -> TestResult {
    for sql in [
        "UPDATE github_auth_profiles SET incarnation='recreated'",
        "UPDATE github_auth_profiles SET active_revision=2",
        "UPDATE github_auth_profiles SET status='Retiring'",
        "DELETE FROM github_auth_profiles",
    ] {
        let test = Fixture::new(true).await?;
        test.seed(true).await?;
        test.sql_during_lookup(sql).await;
        let (status, body) = test.get(URI, Some(fixture::oidc::SCOPES)).await?;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["code"], "AuthProfileChanged");
        assert_eq!(test.calls(), 1);
    }
    Ok(())
}

#[tokio::test]
async fn installation_link_rejects_noncanonical_keys_without_lookup() -> TestResult {
    let test = Fixture::new(true).await?;
    test.seed(true).await?;
    let actor = Actor {
        name: "operator".into(),
        scopes: vec![Scope::AuthRead, Scope::AuthWrite],
    };
    for key in [" shared-github", "shared-github\n", "../shared-github", ""] {
        assert_eq!(
            test.links.installation_link(&actor, key).await,
            Err(Error::NotFound)
        );
    }
    assert_eq!(test.calls(), 0);
    Ok(())
}

#[tokio::test]
async fn installation_link_rejects_missing_credential_without_github_call() -> TestResult {
    let test = Fixture::new(true).await?;
    test.seed(true).await?;
    test.sql("UPDATE github_auth_profile_revisions SET credential_bytes=x''")
        .await?;
    let (status, body) = test.get(URI, Some(fixture::oidc::SCOPES)).await?;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["code"], "GitHubAppUnavailable");
    assert_eq!(test.calls(), 0);
    Ok(())
}
