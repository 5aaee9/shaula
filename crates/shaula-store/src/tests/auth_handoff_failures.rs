//! Real handoff orchestration and SQLite, with remote failure barriers.
#![allow(clippy::unwrap_used)] // Fixed fixture assertions.
#[path = "auth_handoff_failure_port.rs"]
mod remote;
use super::auth_execution::ready;
use super::auth_v2::{binding, org_selector, policy_json, promotion, seed_v2_candidate, PROFILE};
use remote::{FailingGitHub, FailureStage};
use shaula_core::registry::{ChangeView, ControlPlaneStore, MutationFacts};
use shaula_daemon::handoff::{run_handoff, HandoffProgress};
use std::sync::Arc;

#[derive(Clone, Copy)]
enum Mutation {
    Promotion,
    FleetPut,
    Context,
}

async fn mutate(store: &crate::registry_impl::SqliteControlPlane, mutation: Mutation) {
    match mutation {
        Mutation::Promotion => {
            seed_v2_candidate(
                store.store(),
                2,
                &policy_json(&[&org_selector("example-org")]),
            )
            .await;
            let dependents = store.auth_live_dependents(PROFILE).await.unwrap();
            let result = store
                .auth_apply_validation_v2(
                    PROFILE,
                    2,
                    true,
                    None,
                    20,
                    Some(promotion(
                        vec![binding("example-org", 100, 12)],
                        2,
                        &dependents,
                    )),
                )
                .await
                .unwrap();
            assert_eq!(
                result,
                shaula_core::registry::AuthPromotionOutcome::Promoted
            );
        }
        Mutation::FleetPut => {
            let previous = store.fleet_revision_latest("fleet").await.unwrap().unwrap();
            let result = store
                .commit_fleet_mutation(MutationFacts {
                    resource_kind: "fleet",
                    resource_key: "fleet".into(),
                    incarnation: "inc-fleet".into(),
                    revision: 2,
                    spec_json: previous.spec_json,
                    template: None,
                    auth_desired: Some(previous.auth_desired),
                    inputs_digest: previous.inputs_digest,
                    actor: "operator".into(),
                    now: 20,
                    change: ChangeView {
                        id: "put".into(),
                        resource_kind: "fleet".into(),
                        resource_key: "fleet".into(),
                        revision: 2,
                        kind: "Put".into(),
                        state: "Pending".into(),
                        reason: None,
                    },
                    outbox_topic: "fleet.reconcile".into(),
                    outbox_payload: "{}".into(),
                    idempotency: None,
                })
                .await
                .unwrap();
            assert!(result.is_ok());
        }
        Mutation::Context => {
            let row = store
                .fleet_auth_context_get("fleet")
                .await
                .unwrap()
                .unwrap();
            let mut context: shaula_core::auth_context::ResolvedAuthContext =
                serde_json::from_str(row.desired_context_json.as_ref().unwrap()).unwrap();
            context.login = "renamed-org".into();
            let tx = store.store().begin().await.unwrap();
            store
                .store()
                .fleet_auth_context_set_desired_tx(
                    &tx,
                    "fleet",
                    PROFILE,
                    1,
                    Some(&serde_json::to_string(&context).unwrap()),
                    20,
                )
                .await
                .unwrap();
            tx.commit().await.unwrap();
        }
    }
}

async fn fail_after_mutation(stage: FailureStage, mutation: Mutation) {
    let (store, _, _) = ready().await;
    let store = Arc::new(crate::registry_impl::SqliteControlPlane::new(
        store,
        std::path::PathBuf::new(),
    ));
    let remote = Arc::new(FailingGitHub::new(stage));
    let store_port: Arc<dyn ControlPlaneStore> = store.clone();
    let remote_port: Arc<dyn shaula_core::ports::GitHubAccessPort> = remote.clone();
    let identity = shaula_core::github::ScaleSetIdentity {
        target: shaula_core::github::GitHubTarget::organization("example-org").unwrap(),
        runner_group: "Default".into(),
        scale_set_name: "scale".into(),
    };
    let attempt = tokio::spawn(async move {
        run_handoff(
            &store_port,
            "fleet",
            &remote_port,
            &(PROFILE.into(), 1),
            &identity,
            None,
            10,
        )
        .await
        .unwrap()
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), remote.reached.notified())
        .await
        .unwrap();
    mutate(&store, mutation).await;
    let before = store.store().handoff_get("fleet").await.unwrap().unwrap();
    let context_before = store.store().fleet_auth_context_get("fleet").await.unwrap();
    remote.release.notify_one();
    let progress = tokio::time::timeout(std::time::Duration::from_secs(5), attempt)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(progress, HandoffProgress::Retry);
    assert_eq!(
        store.store().handoff_get("fleet").await.unwrap().unwrap(),
        before
    );
    assert_eq!(
        store.store().fleet_auth_context_get("fleet").await.unwrap(),
        context_before
    );
    assert_eq!(before.state, "Pending");
    assert!(before.next_retry_at.is_none());
    assert!(before.observed_revision.is_none());
}

#[tokio::test]
async fn failed_handoff_proof_cannot_block_a_promoted_auth_revision() {
    for stage in [
        FailureStage::Route,
        FailureStage::Ownership,
        FailureStage::Identity,
    ] {
        fail_after_mutation(stage, Mutation::Promotion).await;
    }
}

#[tokio::test]
async fn failed_handoff_proof_cannot_block_a_same_auth_fleet_put() {
    for stage in [
        FailureStage::Route,
        FailureStage::Ownership,
        FailureStage::Identity,
    ] {
        fail_after_mutation(stage, Mutation::FleetPut).await;
    }
}

#[tokio::test]
async fn failed_handoff_proof_cannot_overwrite_a_changed_context_intent() {
    fail_after_mutation(FailureStage::Identity, Mutation::Context).await;
}

#[tokio::test]
async fn current_handoff_failure_persists_bounded_backoff() {
    let (store, expectation, _) = ready().await;
    assert!(store
        .handoff_mark_blocked(
            "fleet",
            &(PROFILE.into(), 1),
            &expectation,
            "PermissionDenied",
            30_010
        )
        .await
        .unwrap());
    let row = store.handoff_get("fleet").await.unwrap().unwrap();
    assert_eq!(row.state, "Blocked");
    assert_eq!(row.attempts, 1);
    assert_eq!(row.next_retry_at, Some(30_010));
    assert_eq!(row.reason.as_deref(), Some("PermissionDenied"));
}
