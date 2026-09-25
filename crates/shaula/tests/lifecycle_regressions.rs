#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod common;
#[path = "common/lifecycle_fakes.rs"]
mod fakes;
#[path = "support/lifecycle_ownership_tests.rs"]
mod ownership_tests;
use common::*;
use shaula_core::{
    lifecycle::GenerationState as G,
    registry::{GenerationRecord, LifecycleStore},
};
use shaula_daemon::supervisor::*;
use std::sync::{atomic::Ordering, Arc};
use tower::ServiceExt;

#[tokio::test]
async fn transaction_rejects_same_profile_revision_switch_with_occupancy() {
    use shaula_core::registry::{ChangeView, ControlPlaneStore, MutationError, MutationFacts};
    let (store, _, _, digest) = setup().await;
    seed_idle(store.as_ref(), &digest).await;
    let head = store.fleet_get("f1").await.unwrap().unwrap();
    let previous = store.fleet_revision_latest("f1").await.unwrap().unwrap();
    let result = store
        .commit_fleet_mutation(MutationFacts {
            idempotency_operation: "v1:PUT",
            authentication: Default::default(),
            resource_kind: "fleet",
            resource_key: "f1".into(),
            incarnation: head.incarnation,
            revision: 2,
            spec_json: previous.spec_json,
            template_pool: Vec::new(),
            template_pool_ref: None,
            template: Some(("k8s-linux".into(), 2, digest, "att-2".into())),
            auth_desired: Some(previous.auth_desired),
            inputs_digest: previous.inputs_digest,
            actor: "ops".into(),
            now: 10,
            change: ChangeView {
                id: "replace".into(),
                resource_kind: "fleet".into(),
                resource_key: "f1".into(),
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
    assert!(matches!(
        result,
        Err(MutationError::RetirementBlocked { .. })
    ));
    assert_eq!(
        store
            .fleet_get("f1")
            .await
            .unwrap()
            .unwrap()
            .desired_revision,
        1
    );
    assert!(store.fleet_change_get("replace").await.unwrap().is_none());
}

#[tokio::test]
async fn guarded_generation_insert_rejects_a_stale_fleet_revision() {
    use shaula_core::registry::{ChangeView, ControlPlaneStore, FleetRuntimeGuard, MutationFacts};

    let (store, _, _, digest) = setup().await;
    let head = store.fleet_get("f1").await.unwrap().unwrap();
    let previous = store.fleet_revision_latest("f1").await.unwrap().unwrap();
    let guard = FleetRuntimeGuard::from(&head);
    let mut spec: serde_json::Value = serde_json::from_str(&previous.spec_json).unwrap();
    spec["capacity"]["max_runners"] = serde_json::json!(10);
    let template = (
        previous.template_profile_key.clone().unwrap(),
        previous.template_revision.unwrap(),
        previous.template_artifact_digest.clone().unwrap(),
        previous.template_attestation_id.clone().unwrap(),
    );
    assert!(matches!(
        store
            .commit_fleet_mutation(MutationFacts {
                idempotency_operation: "v1:PUT",
                authentication: Default::default(),
                resource_kind: "fleet",
                resource_key: "f1".into(),
                incarnation: head.incarnation.clone(),
                revision: 2,
                spec_json: serde_json::to_string(&spec).unwrap(),
                template_pool: previous.template_pool.clone(),
                template_pool_ref: previous.template_pool_ref.clone(),
                template: Some(template.clone()),
                auth_desired: Some(previous.auth_desired.clone()),
                inputs_digest: previous.inputs_digest.clone(),
                actor: "ops".into(),
                now: 20,
                change: ChangeView {
                    id: "stale-head".into(),
                    resource_kind: "fleet".into(),
                    resource_key: "f1".into(),
                    revision: 2,
                    kind: "Replace".into(),
                    state: "Pending".into(),
                    reason: None,
                },
                outbox_topic: "fleet.change".into(),
                outbox_payload: "{}".into(),
                idempotency: None,
            })
            .await
            .unwrap(),
        Ok(())
    ));

    let inserted = store
        .generation_insert_guarded(
            GenerationRecord {
                id: "stale-generation".into(),
                fleet_key: "f1".into(),
                runner_name: "runner-stale".into(),
                generation_name: "generation-stale".into(),
                fleet_revision: 1,
                template_profile_key: template.0,
                pool_member_key: None,
                template_revision: template.1,
                template_artifact_digest: template.2,
                attestation_id: template.3,
                inputs_digest: previous.inputs_digest,
                state: G::CreatePending,
                github_runner_id: None,
                workspace_path: "/tmp/stale-generation".into(),
                created_at: 21,
                updated_at: 21,
            },
            &guard,
        )
        .await
        .unwrap();
    assert!(!inserted);
    assert!(store
        .generation_get("stale-generation")
        .await
        .unwrap()
        .is_none());

    // Keep the fixture's template digest live in the test's setup path.
    assert!(!digest.is_empty());
}

#[path = "support/lifecycle_setup.rs"]
mod setup_fixture;
use setup_fixture::*;

#[tokio::test]
async fn retiring_retries_removal_even_at_zero_excess() {
    let (store, github, supervisor, digest) = setup().await;
    seed_idle(store.as_ref(), &digest).await;
    supervisor.tick(5).await.unwrap(); // G3: settle the Pending handoff first.
    github.busy.store(true, Ordering::SeqCst);
    supervisor.tick(10).await.unwrap();
    assert_eq!(
        store.generation_get("gen1").await.unwrap().unwrap().state,
        G::Retiring
    );
    supervisor.tick(20).await.unwrap();
    assert_eq!(github.removals.load(Ordering::SeqCst), 2);
    assert_eq!(
        store.generation_get("gen1").await.unwrap().unwrap().state,
        G::Retiring
    );
}

#[tokio::test]
async fn destroy_pending_is_reclassified_without_excess() {
    let (store, _, supervisor, digest) = setup().await;
    seed_idle(store.as_ref(), &digest).await;
    supervisor.tick(5).await.unwrap(); // G3: settle the Pending handoff first.
    for state in [G::Retiring, G::DestroyPending] {
        store.generation_advance("gen1", state, 4).await.unwrap();
    }
    supervisor.tick(10).await.unwrap();
    // Missing Create proof is quarantined; it must neither strand nor reach Terraform.
    assert_eq!(
        store.generation_get("gen1").await.unwrap().unwrap().state,
        G::Quarantined
    );
}

#[tokio::test]
async fn cleanup_with_missing_auth_reference_is_quarantined() {
    let (store, github, supervisor, digest) = setup().await;
    supervisor.tick(5).await.unwrap(); // settle the Pending handoff first.
    store
        .generation_insert(GenerationRecord {
            id: "orphan-auth".into(),
            fleet_key: "f1".into(),
            runner_name: "runner-orphan-auth".into(),
            generation_name: "generation-orphan-auth".into(),
            // No matching fleet revision exists, so the destroy path cannot
            // prove which auth client owns the runner removal.
            fleet_revision: 999,
            pool_member_key: None,
            template_profile_key: "k8s-linux".into(),
            template_revision: 1,
            template_artifact_digest: digest,
            attestation_id: "att".into(),
            inputs_digest: "inputs".into(),
            state: G::CreatePending,
            github_runner_id: None,
            workspace_path: "unused".into(),
            created_at: 1,
            updated_at: 1,
        })
        .await
        .unwrap();
    store
        .generation_set_github_runner("orphan-auth", 12, 2)
        .await
        .unwrap();
    for state in [G::Creating, G::WaitingOnline, G::CleanupRequired] {
        store
            .generation_advance("orphan-auth", state, 3)
            .await
            .unwrap();
    }

    // Give the settled listener one effect pass before exercising cleanup.
    supervisor.tick(4).await.unwrap();
    let report = supervisor.tick(60_003).await.unwrap();
    assert_eq!(report.quarantined, 0);
    assert_eq!(github.removals.load(Ordering::SeqCst), 0);
    assert_eq!(
        store
            .generation_get("orphan-auth")
            .await
            .unwrap()
            .unwrap()
            .state,
        G::Quarantined
    );
}

#[tokio::test]
async fn adopted_access_failure_and_unknown_inventory_block_effects() {
    let (store, github, supervisor, _) = setup().await;
    // G3: the first pass settles the Pending handoff; effects reconcile
    // from the acknowledged snapshot on the following pass.
    let settled = supervisor.tick(9).await.unwrap();
    assert!(settled.handoff_acknowledged);
    assert!(supervisor.tick(10).await.unwrap().scale_set_bound);
    github.denied.store(true, Ordering::SeqCst);
    store.demand_snapshot("f1", 1, 11).await.unwrap();
    let blocked = supervisor.tick(20).await.unwrap();
    assert!(blocked.blocked);
    assert_eq!(
        blocked.reason,
        Some(shaula_core::error::ReasonCode::PermissionDenied)
    );
    assert_eq!(
        store.scale_set_get("f1").await.unwrap().unwrap().state,
        "AccessBlocked"
    );
    github.denied.store(false, Ordering::SeqCst);
    github
        .runners
        .lock()
        .unwrap()
        .push(shaula_core::ports::RunnerRef {
            id: 999,
            name: "foreign".into(),
            scale_set_id: 42,
            status: "offline".to_string(),
        });
    let blocked = supervisor.tick(30).await.unwrap();
    assert!(blocked.blocked);
    assert_eq!(
        blocked.reason,
        Some(shaula_core::error::ReasonCode::UnknownRemoteRunner)
    );
    assert_eq!(
        store.scale_set_get("f1").await.unwrap().unwrap().state,
        "UnknownRemoteRunner"
    );
    assert_eq!(github.effects.load(Ordering::SeqCst), 0);
}

/// R4 (spec 0011 §5.3): a stale/unprovable route proof blocks NEW
/// management effects (create-or-adopt and Creates) while safe cleanup of
/// existing generations still proceeds; when the route re-proves, effects
/// resume. No fallback to another credential ever happens.
#[tokio::test]
async fn route_proof_denial_blocks_ownership_and_creates_only() {
    let (store, github, supervisor, _digest) = setup().await;
    // G3: settle the Pending handoff first (settlement tick stops before
    // effects); the next pass reconciles from the acknowledged snapshot.
    let settled = supervisor.tick(9).await.unwrap();
    assert!(settled.handoff_acknowledged);
    assert!(supervisor.tick(10).await.unwrap().scale_set_bound);
    store.demand_snapshot("f1", 1, 11).await.unwrap();

    // Route unprovable: no new ownership effects and no Creates; the tick
    // reports blocked instead of proceeding on stale authorization.
    github.proof_denied.store(true, Ordering::SeqCst);
    let report = supervisor.tick(20).await.unwrap();
    assert!(report.blocked);
    assert!(!report.scale_set_bound);
    assert_eq!(report.created, 0);
    assert_eq!(github.effects.load(Ordering::SeqCst), 0);

    // Route re-proven: create effects are allowed again.
    github.proof_denied.store(false, Ordering::SeqCst);
    let report = supervisor.tick(30).await.unwrap();
    assert!(!report.blocked);
    assert!(report.scale_set_bound);
}

#[path = "support/lifecycle_readiness.rs"]
mod readiness_tests;
