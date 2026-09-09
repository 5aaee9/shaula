//! Continuity and deferral tests for the REAL v2 worker (F2/F8): numeric
//! App identity continuity, different-App rejection, and
//! Retry-After deferral deadlines surfaced from token-mint rate limits.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::auth_worker_v2::tests::{control_plane, policy, real_pem};
use shaula_core::ports::Clock;
use shaula_core::registry::ControlPlaneStore;
use shaula_store::registry_impl::SqliteControlPlane;
use std::sync::Arc;

struct Now;
impl Clock for Now {
    fn now_unix_ms(&self) -> i64 {
        1_800_000_000_000
    }
}

const KEY: &str = "shared-github";

#[tokio::test]
async fn exact_repository_requires_the_installation_account_as_owner() {
    let mock = crate::auth_worker_mock::mock_server(false).await;
    mock.repo_owner_id
        .store(777, std::sync::atomic::Ordering::SeqCst);
    let control_plane = control_plane().await;
    crate::auth_worker_v2::tests::seed_candidate(
        &control_plane,
        1,
        r#"{"selectors":[{"kind":"repository","owner":"5aaee9","repository":"proj"}]}"#,
    )
    .await;
    let store: Arc<dyn ControlPlaneStore> = control_plane;
    let row = store.auth_revision_get(KEY, 1).await.unwrap().unwrap();
    let verdict = crate::auth_worker_v2::validate_v2(
        &store,
        &(Arc::new(Now) as Arc<dyn Clock>),
        KEY,
        &row,
        &crate::auth_worker_mock::endpoints(&mock.base),
    )
    .await
    .unwrap();
    assert_eq!(verdict, crate::auth_worker_v2::Verdict::Rejected);
    assert_eq!(
        store
            .auth_profile_get(KEY)
            .await
            .unwrap()
            .unwrap()
            .active_revision,
        None
    );
    assert!(store.auth_bindings_get(KEY, 1).await.unwrap().is_empty());
}

/// Seeds an active v2 predecessor with its proven numeric App identity.
async fn seed_predecessor(control_plane: &Arc<SqliteControlPlane>, app_id: &str) {
    use shaula_core::registry::{AuthRevisionRow, MutationFacts};
    let change = shaula_core::registry::ChangeView {
        id: "change-predecessor".into(),
        resource_kind: "github_auth_profile".into(),
        resource_key: KEY.into(),
        revision: 1,
        kind: "Rotate".into(),
        state: "Pending".into(),
        reason: None,
    };
    let facts = MutationFacts {
        resource_kind: "github_auth_profile",
        resource_key: KEY.into(),
        incarnation: "inc-v2".into(),
        revision: 1,
        spec_json: String::new(),
        template: None,
        auth_desired: None,
        inputs_digest: String::new(),
        actor: "tester".into(),
        now: 1,
        change,
        outbox_topic: "profile.auth_validate".into(),
        outbox_payload: "{}".into(),
        idempotency: None,
    };
    let credential = AuthRevisionRow {
        profile_key: KEY.into(),
        revision: 1,
        kind: "github_app".into(),
        app_id: Some(app_id.into()),
        schema_version: 2,
        policy_json: Some(policy(true)),
        validation_snapshot_json: None,
        state: "Validating".into(),
        reason: None,
    };
    control_plane
        .commit_auth_revision(facts, credential, real_pem().as_bytes())
        .await
        .unwrap()
        .unwrap();
    let mut promotion = crate::auth_worker_mock::promotion_from(KEY, 1, &[]);
    promotion
        .bindings
        .push(shaula_core::auth_context::AccountBinding {
            account_id: 110,
            account_kind: shaula_core::auth_policy::AccountKind::Organization,
            login: "Indexyz".into(),
            installation_id: 11,
            repository_selection: shaula_core::auth_context::RepositorySelection::All,
            validated_at_ms: 2,
        });
    let mut snapshot: shaula_core::registry::AuthValidationSnapshot =
        serde_json::from_str(&promotion.snapshot_json).unwrap();
    snapshot
        .identities
        .push(shaula_core::registry::AuthIdentityProof {
            login: "Indexyz".into(),
            account_id: 110,
            installation_id: 11,
            repositories: Vec::new(),
        });
    promotion.snapshot_json = serde_json::to_string(&snapshot).unwrap();
    ControlPlaneStore::auth_apply_validation_v2(
        control_plane.as_ref(),
        KEY,
        1,
        true,
        None,
        2,
        Some(promotion),
    )
    .await
    .unwrap();
}

/// The same verified numeric App may add a new account selector.
#[tokio::test]
async fn numeric_app_identity_preserved_across_publications() {
    let mock = crate::auth_worker_mock::mock_server(false).await;
    let control_plane = control_plane().await;
    seed_predecessor(&control_plane, crate::auth_worker_v2::tests::APP_ID).await;

    // The v2 Candidate declares the NUMERIC id of the same App.
    crate::auth_worker_v2::tests::seed_candidate(&control_plane, 2, &policy(false)).await;
    let row = ControlPlaneStore::auth_revision_get(control_plane.as_ref(), KEY, 2)
        .await
        .unwrap()
        .unwrap();
    let store_dyn: Arc<dyn ControlPlaneStore> = control_plane.clone();
    let verdict = crate::auth_worker_v2::validate_v2(
        &store_dyn,
        &(Arc::new(Now) as Arc<dyn Clock>),
        KEY,
        &row,
        &crate::auth_worker_mock::endpoints(&mock.base),
    )
    .await
    .unwrap();
    assert_eq!(
        verdict,
        crate::auth_worker_v2::Verdict::Accepted,
        "same numeric App identity must permit a new selector"
    );
    let head = ControlPlaneStore::auth_profile_get(control_plane.as_ref(), KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(head.active_revision, Some(2));
}

/// A different verified numeric App cannot replace a profile's identity.
#[tokio::test]
async fn different_numeric_app_identity_rejects() {
    let mock = crate::auth_worker_mock::mock_server(false).await;
    let control_plane = control_plane().await;
    seed_predecessor(&control_plane, "1234").await;
    crate::auth_worker_v2::tests::seed_candidate(&control_plane, 2, &policy(false)).await;
    let row = ControlPlaneStore::auth_revision_get(control_plane.as_ref(), KEY, 2)
        .await
        .unwrap()
        .unwrap();
    let store_dyn: Arc<dyn ControlPlaneStore> = control_plane.clone();
    let verdict = crate::auth_worker_v2::validate_v2(
        &store_dyn,
        &(Arc::new(Now) as Arc<dyn Clock>),
        KEY,
        &row,
        &crate::auth_worker_mock::endpoints(&mock.base),
    )
    .await
    .unwrap();
    assert_eq!(verdict, crate::auth_worker_v2::Verdict::Rejected);
    // The predecessor stays active — no silent rebind.
    let head = ControlPlaneStore::auth_profile_get(control_plane.as_ref(), KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(head.active_revision, Some(1));
}

/// F8: a rate-limited token MINT (403 + Retry-After: 45 during the
/// runner probe bootstrap) stays Pending and surfaces the deadline.
#[tokio::test]
async fn rate_limit_at_token_mint_defers_with_deadline() {
    let mock = crate::auth_worker_mock::mock_server_cfg(crate::auth_worker_mock::MockConfig {
        throttle_token_mint: true,
        ..Default::default()
    })
    .await;
    let control_plane = control_plane().await;
    crate::auth_worker_v2::tests::seed_candidate(&control_plane, 1, &policy(false)).await;
    let row = ControlPlaneStore::auth_revision_get(control_plane.as_ref(), KEY, 1)
        .await
        .unwrap()
        .unwrap();
    let store_dyn: Arc<dyn ControlPlaneStore> = control_plane.clone();
    let verdict = crate::auth_worker_v2::validate_v2(
        &store_dyn,
        &(Arc::new(Now) as Arc<dyn Clock>),
        KEY,
        &row,
        &crate::auth_worker_mock::endpoints(&mock.base),
    )
    .await
    .unwrap();
    assert_eq!(
        verdict,
        crate::auth_worker_v2::Verdict::RetryNeeded {
            retry_after_ms: Some(45_000)
        },
        "Retry-After: 45 must surface as the next-attempt deadline"
    );
    let head = ControlPlaneStore::auth_profile_get(control_plane.as_ref(), KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(head.status, "Validating", "Pending, never terminal");
}
