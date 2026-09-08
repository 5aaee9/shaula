//! End-to-end tests of the REAL v2 candidate worker against a scripted
//! GitHub + Actions Service (R1/R2 verification): method/path, User-Agent,
//! App-JWT vs installation-token usage, dynamic selectors, zero
//! repositories, rate-limit handling, and the rejected-Candidate
//! promotion guard. The worker runs with pinned local endpoints so
//! lower-level fixtures cannot bless an endpoint the worker never uses.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use shaula_core::ports::Clock;
use shaula_core::registry::ControlPlaneStore;
use shaula_store::registry_impl::SqliteControlPlane;
use shaula_store::Store;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// A real RSA PEM: the App JWT must actually SIGN (a stub key fails
/// closed as Configuration, exactly like production would).
pub(crate) fn real_pem() -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../shaula-scaleset/tests/fixtures/app.private.pem"),
    )
    .unwrap()
}

pub(crate) struct Now;
impl Clock for Now {
    fn now_unix_ms(&self) -> i64 {
        1_800_000_000_000
    }
}

pub(crate) const APP_ID: &str = "4863460";
pub(crate) const KEY: &str = "shared-github";

pub(crate) fn installation(
    id: i64,
    app_id: i64,
    login: &str,
    selection: &str,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "app_id": app_id,
        "account": {"id": id * 10, "login": login,
            "type": if login == "Indexyz" { "Organization" } else { "User" }},
        "repository_selection": selection,
        "suspended_at": null,
        "permissions": {
            "metadata": "read",
            "administration": "write",
            "organization_self_hosted_runners": "write",
        },
    })
}

pub(crate) fn policy(org_only: bool) -> String {
    let mut selectors = vec![r#"{"kind":"organization","owner":"Indexyz"}"#.to_string()];
    if !org_only {
        selectors.push(
            r#"{"kind":"account_repositories","account_kind":"user","owner":"5aaee9"}"#.to_string(),
        );
    }
    format!(
        r#"{{"selectors":[{selectors}]}}"#,
        selectors = selectors.join(",")
    )
}

pub(crate) async fn seed_candidate(
    control_plane: &SqliteControlPlane,
    revision: i64,
    policy_json: &str,
) {
    seed_candidate_as(control_plane, KEY, revision, policy_json).await;
}
pub(crate) async fn seed_candidate_as(
    control_plane: &SqliteControlPlane,
    key: &str,
    revision: i64,
    policy_json: &str,
) {
    use shaula_core::registry::{AuthRevisionRow, MutationFacts};
    let now = revision;
    let change = shaula_core::registry::ChangeView {
        id: format!("change-{key}-{revision}"),
        resource_kind: "github_auth_profile".into(),
        resource_key: key.into(),
        revision,
        kind: "Publish".into(),
        state: "Pending".into(),
        reason: None,
    };
    let facts = MutationFacts {
        resource_kind: "github_auth_profile",
        resource_key: key.into(),
        incarnation: "inc-v2".into(),
        revision,
        spec_json: String::new(),
        template: None,
        auth_desired: None,
        inputs_digest: String::new(),
        actor: "tester".into(),
        now,
        change,
        outbox_topic: "profile.auth_validate".into(),
        outbox_payload: format!(r#"{{"key":"{key}","revision":{revision}}}"#),
        idempotency: None,
    };
    let credential = AuthRevisionRow {
        profile_key: key.into(),
        revision,
        kind: "github_app".into(),
        app_id: Some(APP_ID.into()),
        installation_id: None,
        pat_principal: None,
        allowlist_json: String::new(),
        schema_version: 2,
        policy_json: Some(policy_json.into()),
        validation_snapshot_json: None,
        state: "Validating".into(),
        reason: None,
    };
    let accepted = control_plane
        .commit_auth_revision(facts, credential, real_pem().as_bytes())
        .await
        .unwrap();
    assert!(accepted.is_ok());
}

pub(crate) async fn control_plane() -> Arc<SqliteControlPlane> {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("worker.db");
    let root = tmp.path().to_path_buf();
    std::mem::forget(tmp);
    let store = Store::open(&path).await.unwrap();
    store.migrate().await.unwrap();
    Arc::new(SqliteControlPlane::new(store, root))
}

pub(crate) async fn run_worker(
    control_plane: &Arc<SqliteControlPlane>,
    mock: &crate::auth_worker_mock::Mock,
) -> shaula_core::error::CoreResult<crate::auth_worker_v2::Verdict> {
    let row = ControlPlaneStore::auth_revision_get(control_plane.as_ref(), KEY, 1)
        .await
        .unwrap()
        .unwrap();
    let store_dyn: Arc<dyn ControlPlaneStore> = control_plane.clone();
    crate::auth_worker_v2::validate_v2(
        &store_dyn,
        &(Arc::new(Now) as Arc<dyn Clock>),
        KEY,
        &row,
        &crate::auth_worker_mock::endpoints(&mock.base),
    )
    .await
}

/// R1: the happy path drives the REAL worker — `/app` carries the fixed
/// User-Agent, the dynamic selector mints an installation token and reads
/// the installation-token metadata endpoint, and promotion freezes both
/// bindings atomically.
#[tokio::test]
async fn worker_promotes_with_user_agent_and_installation_scoped_metadata() {
    let mock = crate::auth_worker_mock::mock_server(false).await;
    let control_plane = control_plane().await;
    seed_candidate(&control_plane, 1, &policy(false)).await;
    let verdict = run_worker(&control_plane, &mock).await.unwrap();
    assert_eq!(verdict, crate::auth_worker_v2::Verdict::Accepted);
    let head = ControlPlaneStore::auth_profile_get(control_plane.as_ref(), KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(head.active_revision, Some(1));
    let bindings = ControlPlaneStore::auth_bindings_get(control_plane.as_ref(), KEY, 1)
        .await
        .unwrap();
    assert_eq!(bindings.len(), 2);
    assert!(bindings.iter().any(|b| b.login == "Indexyz"));
    assert!(bindings.iter().any(|b| b.login == "5aaee9"));
    // User-Agent MUST be present on App-JWT requests (R1).
    let uas = mock.user_agents.lock().unwrap();
    assert!(
        !uas.is_empty() && uas.iter().all(|ua| !ua.is_empty()),
        "GitHub refuses unattributed clients"
    );
    // The installation-token metadata endpoint was actually minted+used.
    assert!(mock.metadata_mints.load(Ordering::SeqCst) >= 1);
}

/// R1/F8: a rate-limited `403` (x-ratelimit-remaining: 0 + Retry-After)
/// on discovery stays Pending — never a terminal rejection.
#[tokio::test]
async fn worker_rate_limit_stays_pending_not_rejected() {
    let mock = crate::auth_worker_mock::mock_server(false).await;
    let control_plane = control_plane().await;
    seed_candidate(
        &control_plane,
        1,
        r#"{"selectors":[{"kind":"account_repositories","account_kind":"user","owner":"nolimit"}]}"#,
    )
    .await;
    let verdict = run_worker(&control_plane, &mock).await.unwrap();
    assert_eq!(
        verdict,
        crate::auth_worker_v2::Verdict::RetryNeeded {
            retry_after_ms: Some(120_000)
        },
        "Retry-After must surface as the next-attempt deadline"
    );
    let head = ControlPlaneStore::auth_profile_get(control_plane.as_ref(), KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        head.status, "Validating",
        "throttled discovery stays Pending"
    );
}

/// R2: a denied runner probe on an org selector with ZERO dependent
/// fleets terminally rejects the Candidate — it can never activate.
#[tokio::test]
async fn worker_org_selector_probe_denial_rejects_without_fleets() {
    let mock = crate::auth_worker_mock::mock_server(true).await;
    let control_plane = control_plane().await;
    seed_candidate(&control_plane, 1, &policy(true)).await;
    let verdict = run_worker(&control_plane, &mock).await.unwrap();
    assert_eq!(verdict, crate::auth_worker_v2::Verdict::Rejected);
    let head = ControlPlaneStore::auth_profile_get(control_plane.as_ref(), KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(head.active_revision, None, "denied probe never activates");
    let change =
        ControlPlaneStore::profile_change_get(control_plane.as_ref(), &format!("change-{KEY}-1"))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(change.state, "Rejected");
    assert_eq!(change.reason.as_deref(), Some("PermissionDenied"));
    assert!(
        ControlPlaneStore::auth_bindings_get(control_plane.as_ref(), KEY, 1)
            .await
            .unwrap()
            .is_empty()
    );
}

/// R2: a REJECTED desired Candidate can never be promoted afterwards —
/// the store refuses the illegal Candidate→Active transition.
#[tokio::test]
async fn rejected_candidate_cannot_be_promoted_later() {
    let mock = crate::auth_worker_mock::mock_server(true).await;
    let control_plane = control_plane().await;
    seed_candidate(&control_plane, 1, &policy(true)).await;
    let verdict = run_worker(&control_plane, &mock).await.unwrap();
    assert_eq!(verdict, crate::auth_worker_v2::Verdict::Rejected);
    // A late validation callback claiming success must fail, never flip
    // the rejected revision Active.
    let outcome = ControlPlaneStore::auth_apply_validation_v2(
        control_plane.as_ref(),
        KEY,
        1,
        true,
        None,
        99,
        Some(crate::auth_worker_mock::promotion_from(KEY, 1, &[])),
    )
    .await;
    assert!(outcome.is_err(), "rejected candidate promotion refused");
    let change =
        ControlPlaneStore::profile_change_get(control_plane.as_ref(), &format!("change-{KEY}-1"))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(change.state, "Rejected");
    assert!(
        ControlPlaneStore::auth_profile_get(control_plane.as_ref(), KEY)
            .await
            .unwrap()
            .unwrap()
            .active_revision
            .is_none()
    );
}
