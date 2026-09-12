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
            resource_kind: "fleet",
            resource_key: "f1".into(),
            incarnation: head.incarnation,
            revision: 2,
            spec_json: previous.spec_json,
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

async fn setup() -> (
    Arc<shaula_store::registry_impl::SqliteControlPlane>,
    Arc<fakes::GitHub>,
    FleetSupervisor,
    String,
) {
    setup_with_labels(vec![shaula_core::github::Label {
        name: "shaula-x64".into(),
        label_type: "System".into(),
    }])
    .await
}

async fn setup_with_labels(
    labels: Vec<shaula_core::github::Label>,
) -> (
    Arc<shaula_store::registry_impl::SqliteControlPlane>,
    Arc<fakes::GitHub>,
    FleetSupervisor,
    String,
) {
    let (app, store, engine, service, root) = {
        let (a, s, e, r) = build_app_with_artifact_root().await;
        let cp = shaula_daemon::service::ControlPlane::new(
            s.clone(),
            fixed_clock(),
            b"k".to_vec(),
            100,
            e.clone(),
        );
        (a, s, e, cp, r)
    };
    let digest = common::attestation_harness::seed_profile(&app, &store, "k8s-linux", true).await;
    let body = attest_body(
        &store,
        &digest,
        &expected_bindings_digest("k8s-linux", 1),
        &engine,
    )
    .await;
    assert_eq!(
        common::attestation_harness::put_attestation(&app, "k8s-linux", 1, "att", body)
            .await
            .0,
        axum::http::StatusCode::CREATED
    );
    assert_eq!(
        app.oneshot(authorized(
            "PUT",
            "/api/v1/fleets/f1",
            Some(FLEET_BODY.into())
        ))
        .await
        .unwrap()
        .status(),
        axum::http::StatusCode::ACCEPTED
    );
    let github = Arc::new(fakes::GitHub::default());
    let supervisor = FleetSupervisor::new(
        FleetSupervisorDeps {
            limits: LifecycleLimits {
                create: Arc::new(tokio::sync::Semaphore::new(1)),
                destroy: Arc::new(tokio::sync::Semaphore::new(1)),
            },
            store: store.clone(),
            handoff: store.clone(),
            github: github.clone(),
            runtime: Arc::new(GatedRuntime),
        },
        FleetSupervisorConfig {
            fleet_key: "f1".into(),
            capacity: shaula_core::capacity::CapacityPolicy {
                min_runners: 0,
                max_runners: 5,
            },
            work_root: root.join("work"),
            artifact_root: root,
            operation_timeout: std::time::Duration::from_secs(1),
            apply_intent_sink: Arc::new(shaula_daemon::apply_intent::LedgerApplyIntentSink {
                store: store.clone(),
                gates: service.effect_gates(),
            }),
            labels,
            auth_profile_key: "prod-app".into(),
            auth_revision: 1,
        },
        shaula_core::github::ScaleSetIdentity {
            target: shaula_core::github::GitHubTarget::organization("example-org").unwrap(),
            runner_group: "Default".into(),
            scale_set_name: "shaula-x64".into(),
        },
    );
    (store, github, supervisor, digest)
}

struct GatedRuntime;
#[async_trait::async_trait]
impl shaula_core::ports::TemplateRuntimePort for GatedRuntime {
    async fn create(
        &self,
        _: shaula_core::ports::TemplateCreateRequest,
    ) -> Result<shaula_core::ports::TemplateCreateResult, shaula_core::ports::TemplateOutcomeError>
    {
        panic!("Create must be gated")
    }
    async fn destroy(
        &self,
        _: shaula_core::ports::TemplateDestroyRequest,
    ) -> Result<shaula_core::ports::DestroyClassification, shaula_core::ports::TemplateOutcomeError>
    {
        panic!("Destroy must be gated")
    }
}

async fn seed_idle(store: &impl LifecycleStore, digest: &str) {
    store
        .generation_insert(GenerationRecord {
            id: "gen1".into(),
            fleet_key: "f1".into(),
            runner_name: "runner1".into(),
            generation_name: "generation1".into(),
            fleet_revision: 1,
            template_profile_key: "k8s-linux".into(),
            template_revision: 1,
            template_artifact_digest: digest.into(),
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
        .generation_set_github_runner("gen1", 12, 2)
        .await
        .unwrap();
    for state in [G::Creating, G::WaitingOnline, G::Idle] {
        store.generation_advance("gen1", state, 3).await.unwrap();
    }
}

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

async fn seed_waiting_online(store: &impl LifecycleStore, digest: &str) {
    store
        .generation_insert(GenerationRecord {
            id: "gen1".into(),
            fleet_key: "f1".into(),
            runner_name: "runner1".into(),
            generation_name: "generation1".into(),
            fleet_revision: 1,
            template_profile_key: "k8s-linux".into(),
            template_revision: 1,
            template_artifact_digest: digest.into(),
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
        .generation_set_github_runner("gen1", 12, 2)
        .await
        .unwrap();
    for state in [G::Creating, G::WaitingOnline] {
        store.generation_advance("gen1", state, 3).await.unwrap();
    }
}

async fn generation_state(store: &impl LifecycleStore) -> G {
    store
        .generations_for_fleet("f1")
        .await
        .unwrap()
        .pop()
        .unwrap()
        .state
}

#[tokio::test]
async fn readiness_observed_online_runner_advences_waiting_generation_to_idle() {
    let (store, github, supervisor, digest) = setup().await;
    seed_waiting_online(store.as_ref(), &digest).await;
    supervisor.tick(5).await.unwrap(); // settle the Pending handoff.
    github
        .runners
        .lock()
        .unwrap()
        .push(shaula_core::ports::RunnerRef {
            id: 12,
            name: "runner1".into(),
            scale_set_id: 42,
            status: "online".into(),
        });
    // Busy gates runner removal, so retirement engages (proving Idle was
    // reached) without driving a gated destroy in this harness.
    github.busy.store(true, Ordering::SeqCst);
    supervisor.tick(30).await.unwrap();
    assert_eq!(generation_state(store.as_ref()).await, G::Retiring);
    assert_eq!(github.removals.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn readiness_timeout_moves_absent_runner_to_cleanup() {
    let (store, github, supervisor, digest) = setup().await;
    seed_waiting_online(store.as_ref(), &digest).await;
    supervisor.tick(5).await.unwrap();
    // No runner in the inventory (never registered, or an ephemeral JIT
    // runner that served its job and self-deregistered) and past the
    // grace period (spec 0024 §2).
    let late = 1 + shaula_daemon::supervisor::READINESS_TIMEOUT_MS + 1;
    supervisor.tick(late).await.unwrap();
    assert_eq!(generation_state(store.as_ref()).await, G::CleanupRequired);
    assert_eq!(github.removals.load(Ordering::SeqCst), 0);
}

/// 2026-09-12 incident: a JIT ephemeral runner observed online (Idle)
/// then served its job and self-deregistered. With demand still >= 1
/// (the next job already assigned to the scale set), the phantom Idle
/// generation held effective capacity forever: nothing created a
/// replacement and nothing destroyed the dead VM. The Idle inventory
/// re-check must advance it to Retiring so the removal/destroy chain
/// runs even at zero excess (rev 3, spec 0024).
#[tokio::test]
async fn idle_generation_whose_runner_vanished_from_inventory_retires() {
    let (store, github, supervisor, digest) = setup().await;
    seed_idle(store.as_ref(), &digest).await;
    supervisor.tick(5).await.unwrap(); // settle the Pending handoff.
                                       // Demand snapshot keeps target at 1 while the runner is already
                                       // gone from the inventory (self-deregistered after its single job).
    store.demand_snapshot("f1", 1, 6).await.unwrap();
    supervisor.tick(10).await.unwrap();
    // Idle -> Retiring, then the retirement chain re-gates the runner
    // removal at zero excess and attempts it once (the seeded row has
    // no Create provenance, so the destroy gate quarantines it rather
    // than reaching Terraform).
    assert_eq!(
        store.generation_get("gen1").await.unwrap().unwrap().state,
        G::Quarantined
    );
    assert_eq!(github.removals.load(Ordering::SeqCst), 1);
}

/// An Idle generation whose runner is still online in the inventory is
/// untouched: the phantom-runner re-check must never retire live
/// capacity.
#[tokio::test]
async fn idle_generation_with_online_runner_stays_idle() {
    let (store, github, supervisor, digest) = setup().await;
    seed_idle(store.as_ref(), &digest).await;
    supervisor.tick(5).await.unwrap(); // settle the Pending handoff.
    github
        .runners
        .lock()
        .unwrap()
        .push(shaula_core::ports::RunnerRef {
            id: 12,
            name: "runner1".into(),
            scale_set_id: 42,
            status: "online".into(),
        });
    store.demand_snapshot("f1", 1, 6).await.unwrap();
    supervisor.tick(10).await.unwrap();
    assert_eq!(
        store.generation_get("gen1").await.unwrap().unwrap().state,
        G::Idle
    );
    assert_eq!(github.removals.load(Ordering::SeqCst), 0);
}

/// An Idle generation whose runner is still registered but offline
/// (agent dead, unit is Restart=no) can never serve another job — it
/// retires rather than stranding capacity on a zombie registration.
#[tokio::test]
async fn idle_generation_with_offline_runner_retires() {
    let (store, github, supervisor, digest) = setup().await;
    seed_idle(store.as_ref(), &digest).await;
    supervisor.tick(5).await.unwrap(); // settle the Pending handoff.
    github
        .runners
        .lock()
        .unwrap()
        .push(shaula_core::ports::RunnerRef {
            id: 12,
            name: "runner1".into(),
            scale_set_id: 42,
            status: "offline".into(),
        });
    store.demand_snapshot("f1", 1, 6).await.unwrap();
    supervisor.tick(10).await.unwrap();
    assert_eq!(
        store.generation_get("gen1").await.unwrap().unwrap().state,
        G::Quarantined
    );
    assert_eq!(github.removals.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn readiness_keeps_young_absent_generation_waiting() {
    let (store, _github, supervisor, digest) = setup().await;
    seed_waiting_online(store.as_ref(), &digest).await;
    supervisor.tick(5).await.unwrap();
    // Within the boot grace: no transition, no removal, no cleanup.
    supervisor.tick(5 * 60 * 1000).await.unwrap();
    assert_eq!(generation_state(store.as_ref()).await, G::WaitingOnline);
}
