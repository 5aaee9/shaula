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
        label_type: "Customer".into(),
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
