use super::fixture::Fixture;
use shaula_core::ports::{AccessFailure, GitHubAccessPort};
use std::sync::atomic::Ordering;

#[tokio::test]
async fn drifted_repository_identity_fails_the_refresh() {
    let f = Fixture::start().await;
    let client = f.client(true, false);
    client.ensure_route_proof().await.unwrap();
    f.script.repository.write().unwrap()["id"] = 999.into();
    f.advance(60_000);
    assert!(matches!(
        client.ensure_route_proof().await,
        Err(AccessFailure::PermissionDenied)
    ));
}

#[tokio::test]
async fn legacy_client_id_credential_keeps_reconciling() {
    let f = Fixture::start().await;
    let proof = f
        .client_with_issuer(false, false, "Iv23legacy")
        .ensure_route_proof()
        .await
        .unwrap();
    assert_eq!(proof.installation_id, 34);
    assert_eq!(proof.account_id, 100);
    assert_eq!(proof.organization_id, Some(100));
}

#[tokio::test]
async fn rebinding_expected_context_cannot_reuse_the_old_positive_cache() {
    let f = Fixture::start().await;
    let client = f.client(true, true);
    client.ensure_route_proof().await.unwrap();
    let mut expected = client.expected_context.clone().unwrap();
    expected.repository_id = Some(999);
    let client = client.with_expected_context(Some(expected));
    assert!(matches!(
        client.ensure_route_proof().await,
        Err(AccessFailure::PermissionDenied)
    ));
    assert_eq!(f.reads(), 2);
}

#[tokio::test]
async fn first_proof_requires_complete_metadata_matching_persisted_identity() {
    for mutation in [
        "missing_owner",
        "owner_float",
        "owner_zero",
        "unsafe_repo",
        "repo_recreated",
        "owner_changed",
        "login_changed",
        "repo_name_changed",
    ] {
        let f = Fixture::start().await;
        let client = f.client(true, true);
        {
            let mut body = f.script.repository.write().unwrap();
            match mutation {
                "missing_owner" => {
                    body["owner"].as_object_mut().unwrap().remove("id");
                }
                "owner_float" => body["owner"]["id"] = 100.0.into(),
                "owner_zero" => body["owner"]["id"] = 0.into(),
                "unsafe_repo" => body["id"] = (1_i64 << 53).into(),
                "repo_recreated" => body["id"] = 701.into(),
                "owner_changed" => body["owner"]["id"] = 101.into(),
                "login_changed" => body["owner"]["login"] = "other".into(),
                "repo_name_changed" => body["name"] = "other".into(),
                _ => unreachable!(),
            }
        }
        super::assert_new_effects_blocked(&f, &client).await;
        assert_eq!(f.script.effects.load(Ordering::SeqCst), 0, "{mutation}");
        assert_eq!(f.reads(), 1, "negative cache after {mutation}");
    }
}

#[tokio::test]
async fn persisted_context_cannot_omit_pins_or_change_any_principal_dimension() {
    for mutation in [
        "owner_pin",
        "repo_pin",
        "app",
        "account",
        "installation",
        "kind",
        "host",
        "target",
    ] {
        let f = Fixture::start().await;
        let mut client = f.client(true, true);
        let expected = client.expected_context.as_mut().unwrap();
        match mutation {
            "owner_pin" => expected.repository_owner_id = None,
            "repo_pin" => expected.repository_id = None,
            "app" => expected.app_id = "124".into(),
            "account" => expected.account_id = 101,
            "installation" => expected.installation_id = 35,
            "kind" => expected.account_kind = shaula_core::auth_policy::AccountKind::User,
            "host" => expected.github_host = "other.example".into(),
            "target" => {
                expected.target =
                    shaula_core::github::GitHubTarget::new_repository("example", "other").unwrap()
            }
            _ => unreachable!(),
        }
        assert!(
            matches!(
                client.ensure_route_proof().await,
                Err(AccessFailure::PermissionDenied)
            ),
            "{mutation}"
        );
        assert_eq!(f.script.effects.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn installation_owner_and_required_permission_are_rechecked() {
    for mutation in ["account", "kind", "permission", "suspended", "unsafe_app"] {
        let f = Fixture::start().await;
        let client = f.client(false, true);
        client.ensure_route_proof().await.unwrap();
        {
            let mut body = f.script.installation.write().unwrap();
            match mutation {
                "account" => body["account"]["id"] = 101.into(),
                "kind" => body["account"]["type"] = "User".into(),
                "permission" => {
                    body["permissions"]["organization_self_hosted_runners"] = "read".into()
                }
                "suspended" => body["suspended_at"] = "2026-09-07T00:00:00Z".into(),
                "unsafe_app" => body["app_id"] = (1_i64 << 53).into(),
                _ => unreachable!(),
            }
        }
        f.advance(60_000);
        super::assert_new_effects_blocked(&f, &client).await;
        assert_eq!(f.script.effects.load(Ordering::SeqCst), 0, "{mutation}");
    }
}
