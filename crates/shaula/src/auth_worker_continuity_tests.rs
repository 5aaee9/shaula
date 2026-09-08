//! Continuity and deferral tests for the REAL v2 worker (F2/F8): legacy
//! client-ID → numeric-App continuity, different-App rejection, and
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

/// Seeds a LEGACY (schema 1) active predecessor whose stored App identity
/// is a CLIENT-ID string — the historical encoding (spec 0011 §7.5).
async fn seed_legacy_predecessor(control_plane: &Arc<SqliteControlPlane>, app_id: &str) {
    use shaula_core::registry::{AuthRevisionRow, MutationFacts};
    let change = shaula_core::registry::ChangeView {
        id: "change-legacy".into(),
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
        installation_id: Some(34),
        pat_principal: None,
        allowlist_json: r#"{"targets":[{"kind":"organization","owner":"Indexyz"}]}"#.into(),
        schema_version: 1,
        policy_json: None,
        validation_snapshot_json: None,
        state: "Validating".into(),
        reason: None,
    };
    control_plane
        .commit_auth_revision(facts, credential, real_pem().as_bytes())
        .await
        .unwrap()
        .unwrap();
    // Legacy staged activation (no bindings/snapshot on this path).
    ControlPlaneStore::auth_apply_validation_v2(
        control_plane.as_ref(),
        KEY,
        1,
        true,
        None,
        2,
        None,
    )
    .await
    .unwrap();
}

/// F2: legacy client-ID predecessor + `/app` client_id MATCH → the same
/// App upgrades; continuity holds.
#[tokio::test]
async fn legacy_client_id_same_app_upgrades() {
    let mock = crate::auth_worker_mock::mock_server_cfg(crate::auth_worker_mock::MockConfig {
        client_id: "Iv23legacy",
        ..Default::default()
    })
    .await;
    let control_plane = control_plane().await;
    seed_legacy_predecessor(&control_plane, "Iv23legacy").await;

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
        "same-App continuity through /app client_id must succeed"
    );
    let head = ControlPlaneStore::auth_profile_get(control_plane.as_ref(), KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(head.active_revision, Some(2));
}

/// F2: a DIFFERENT App (client_id mismatch on `/app`) can never replace
/// the profile under the same key — the predecessor identity is enforced
/// even though the legacy row has no v2 snapshot.
#[tokio::test]
async fn legacy_client_id_different_app_rejects() {
    let mock = crate::auth_worker_mock::mock_server_cfg(crate::auth_worker_mock::MockConfig {
        client_id: "Iv23other",
        ..Default::default()
    })
    .await;
    let control_plane = control_plane().await;
    seed_legacy_predecessor(&control_plane, "Iv23legacy").await;
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
    // The LEGACY predecessor stays active — no silent rebind.
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
