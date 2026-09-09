//! Exercise real handoff orchestration with refused durable completion.
#![allow(clippy::unwrap_used)] // Assertions use fixed, non-secret test fixtures.
use super::handoff_tests::{HealthyGitHub, MemoryStore};
use super::{run_handoff, HandoffProgress};
use shaula_core::ports::GitHubAccessPort;
use shaula_core::registry::{AuthHandoffRow, ControlPlaneStore, FleetAuthContextRow};
use std::sync::{atomic::Ordering, Arc};

async fn prepared() -> (Arc<MemoryStore>, shaula_core::github::ScaleSetIdentity) {
    let store = Arc::new(MemoryStore::default());
    let desired = ("profile".into(), 2);
    store.handoffs.lock().await.insert(
        "fleet".into(),
        AuthHandoffRow {
            fleet_key: "fleet".into(),
            desired: desired.clone(),
            observed: None,
            state: "Pending".into(),
            cleanup_only: false,
            blocked_reason: None,
            retry_at: None,
        },
    );
    let target = shaula_core::github::GitHubTarget::organization("example-org").unwrap();
    let context = shaula_core::auth_context::ResolvedAuthContext {
        profile_key: "profile".into(),
        revision: 2,
        github_host: "github.com".into(),
        app_id: "1".into(),
        account_id: 1,
        account_kind: shaula_core::auth_policy::AccountKind::Organization,
        login: "example-org".into(),
        installation_id: 1,
        target: target.clone(),
        organization_id: Some(1),
        repository_id: None,
        repository_owner_id: None,
    };
    *store.context.lock().await = Some(FleetAuthContextRow {
        fleet_key: "fleet".into(),
        desired: Some(desired),
        desired_context_json: Some(serde_json::to_string(&context).unwrap()),
        observed: None,
        observed_context_json: None,
        state: "Pending".into(),
        reason: None,
    });
    (
        store,
        shaula_core::github::ScaleSetIdentity {
            target,
            runner_group: "Default".into(),
            scale_set_name: "scale".into(),
        },
    )
}

#[tokio::test]
async fn captured_client_revision_cannot_prove_a_new_desired_revision() {
    let (store, identity) = prepared().await;
    let github = Arc::new(HealthyGitHub::default());
    let github_port: Arc<dyn GitHubAccessPort> = github.clone();
    let store_port: Arc<dyn ControlPlaneStore> = store.clone();
    let progress = run_handoff(
        &store_port,
        "fleet",
        &github_port,
        &("profile".into(), 1),
        &identity,
        None,
        100,
    )
    .await
    .unwrap();
    assert_eq!(progress, HandoffProgress::Retry);
    assert_eq!(github.proof_calls.load(Ordering::SeqCst), 0);
    assert!(store
        .handoff_get("fleet")
        .await
        .unwrap()
        .unwrap()
        .observed
        .is_none());
}

#[tokio::test]
async fn v2_stale_context_acknowledgement_is_retry_not_settlement() {
    let (store, identity) = prepared().await;
    store.stale_ack.store(true, Ordering::SeqCst);
    let github = Arc::new(HealthyGitHub::default());
    let github_port: Arc<dyn GitHubAccessPort> = github.clone();
    let store_port: Arc<dyn ControlPlaneStore> = store.clone();
    let progress = run_handoff(
        &store_port,
        "fleet",
        &github_port,
        &("profile".into(), 2),
        &identity,
        None,
        100,
    )
    .await
    .unwrap();
    assert_eq!(progress, HandoffProgress::Retry);
    assert_eq!(github.proof_calls.load(Ordering::SeqCst), 1);
    assert!(store
        .handoff_get("fleet")
        .await
        .unwrap()
        .unwrap()
        .observed
        .is_none());
}

#[tokio::test]
async fn unsupported_revision_never_settles_or_reaches_github() {
    for (schema, kind) in [(1, "github_app"), (3, "github_app"), (2, "pat")] {
        let (store, identity) = prepared().await;
        *store.revision_kind.lock().await = Some((schema, kind.into()));
        // Even matching ref tuples plus a complete old context cannot settle.
        let mut handoffs = store.handoffs.lock().await;
        let handoff = handoffs.get_mut("fleet").unwrap();
        handoff.observed = Some(handoff.desired.clone());
        drop(handoffs);
        let github = Arc::new(HealthyGitHub::default());
        let github_port: Arc<dyn GitHubAccessPort> = github.clone();
        let store_port: Arc<dyn ControlPlaneStore> = store.clone();
        let progress = run_handoff(
            &store_port,
            "fleet",
            &github_port,
            &("profile".into(), 2),
            &identity,
            None,
            100,
        )
        .await
        .unwrap();
        assert_eq!(progress, HandoffProgress::Blocked);
        assert_eq!(github.proof_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            store
                .handoff_get("fleet")
                .await
                .unwrap()
                .unwrap()
                .blocked_reason
                .as_deref(),
            Some("UnsupportedAuthenticationRevision")
        );
    }
}

#[tokio::test]
async fn missing_desired_context_never_acknowledges_a_ref_only_handoff() {
    let (store, identity) = prepared().await;
    *store.context.lock().await = None;
    let github: Arc<dyn GitHubAccessPort> = Arc::new(HealthyGitHub::default());
    let store_port: Arc<dyn ControlPlaneStore> = store.clone();
    let progress = run_handoff(
        &store_port,
        "fleet",
        &github,
        &("profile".into(), 2),
        &identity,
        None,
        100,
    )
    .await
    .unwrap();
    assert_eq!(progress, HandoffProgress::Blocked);
    let handoff = store.handoff_get("fleet").await.unwrap().unwrap();
    assert!(handoff.observed.is_none());
    assert_eq!(
        handoff.blocked_reason.as_deref(),
        Some("AuthContextMissing")
    );
}

async fn retain_observed_context(store: &MemoryStore, handoff_state: &str, context_state: &str) {
    let mut handoffs = store.handoffs.lock().await;
    let handoff = handoffs.get_mut("fleet").unwrap();
    handoff.observed = Some(handoff.desired.clone());
    handoff.state = handoff_state.into();
    drop(handoffs);
    let mut context = store.context.lock().await;
    let context = context.as_mut().unwrap();
    context.observed = context.desired.clone();
    context.observed_context_json = context.desired_context_json.clone();
    context.state = context_state.into();
}

#[tokio::test]
async fn matching_refs_only_skip_proof_when_both_states_are_observed() {
    for (handoff_state, context_state, expected, calls) in [
        ("Pending", "Observed", HandoffProgress::Acknowledged, 1),
        ("Observed", "Pending", HandoffProgress::Acknowledged, 1),
        ("Observed", "Observed", HandoffProgress::UpToDate, 0),
    ] {
        let (store, identity) = prepared().await;
        retain_observed_context(&store, handoff_state, context_state).await;
        let github = Arc::new(HealthyGitHub::default());
        let github_port: Arc<dyn GitHubAccessPort> = github.clone();
        let store_port: Arc<dyn ControlPlaneStore> = store;
        let progress = run_handoff(
            &store_port,
            "fleet",
            &github_port,
            &("profile".into(), 2),
            &identity,
            None,
            100,
        )
        .await
        .unwrap();
        assert_eq!(progress, expected, "{handoff_state}/{context_state}");
        assert_eq!(github.proof_calls.load(Ordering::SeqCst), calls);
    }
}

#[tokio::test]
async fn matching_refs_preserve_blocked_backoff_before_reverification() {
    let (store, identity) = prepared().await;
    retain_observed_context(&store, "Blocked", "Observed").await;
    store
        .handoffs
        .lock()
        .await
        .get_mut("fleet")
        .unwrap()
        .retry_at = Some(200);
    let github = Arc::new(HealthyGitHub::default());
    let github_port: Arc<dyn GitHubAccessPort> = github.clone();
    let store_port: Arc<dyn ControlPlaneStore> = store;
    for (now, expected, calls) in [
        (100, HandoffProgress::Blocked, 0),
        (201, HandoffProgress::Acknowledged, 1),
    ] {
        let progress = run_handoff(
            &store_port,
            "fleet",
            &github_port,
            &("profile".into(), 2),
            &identity,
            None,
            now,
        )
        .await
        .unwrap();
        assert_eq!(progress, expected);
        assert_eq!(github.proof_calls.load(Ordering::SeqCst), calls);
    }
}

#[tokio::test]
async fn matching_refs_still_require_a_current_acknowledgement_fence() {
    let (store, identity) = prepared().await;
    retain_observed_context(&store, "Pending", "Observed").await;
    store.stale_ack.store(true, Ordering::SeqCst);
    let github = Arc::new(HealthyGitHub::default());
    let github_port: Arc<dyn GitHubAccessPort> = github.clone();
    let store_port: Arc<dyn ControlPlaneStore> = store.clone();
    let progress = run_handoff(
        &store_port,
        "fleet",
        &github_port,
        &("profile".into(), 2),
        &identity,
        None,
        100,
    )
    .await
    .unwrap();
    assert_eq!(progress, HandoffProgress::Retry);
    assert_eq!(github.proof_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        store.handoff_get("fleet").await.unwrap().unwrap().state,
        "Pending"
    );
}
