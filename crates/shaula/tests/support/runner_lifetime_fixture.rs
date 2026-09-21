use super::{common, fakes};
use common::*;
use shaula_core::{
    lifecycle::GenerationState as G,
    ports::*,
    registry::{GenerationRecord, LifecycleStore},
};
use shaula_daemon::{apply_intent::LedgerApplyIntentSink, supervisor::*};
use shaula_store::registry_impl::SqliteControlPlane;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tower::ServiceExt;

pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

pub struct Clock(pub AtomicI64);
impl shaula_core::ports::Clock for Clock {
    fn now_unix_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

pub struct Runtime {
    pub destroys: AtomicUsize,
    pub fail: AtomicBool,
    pub clock: Arc<Clock>,
}
#[async_trait::async_trait]
impl TemplateRuntimePort for Runtime {
    async fn create(
        &self,
        request: TemplateCreateRequest,
    ) -> Result<TemplateCreateResult, TemplateOutcomeError> {
        let provenance = provenance(&request.input.generation.id);
        let sink = request.apply_intent_sink.ok_or_else(error)?;
        drop(
            sink.persist_apply_starting(&provenance)
                .await
                .map_err(|_| error())?,
        );
        self.clock.0.fetch_add(5_000, Ordering::SeqCst);
        Ok(TemplateCreateResult {
            result_envelope: shaula_core::template::ShaulaResultEnvelope {
                contract_version: 1,
                generation_id: request.input.generation.id,
                bindings_digest: request.expected_bindings_digest,
                resources: vec![],
            },
            state_lineage: "lineage".into(),
            state_serial: 1,
            provenance,
        })
    }
    async fn destroy(
        &self,
        request: TemplateDestroyRequest,
    ) -> Result<DestroyClassification, TemplateOutcomeError> {
        self.destroys.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request.original_state.lineage, "lineage");
        assert_eq!(request.original_state.serial, 1);
        let mut provenance = request.original_provenance;
        provenance.intent = shaula_core::plan::PlanIntent::Destroy;
        provenance.attempt_id = shaula_core::auth::new_attempt_id();
        let sink = request.apply_intent_sink.ok_or_else(error)?;
        drop(
            sink.persist_apply_starting(&provenance)
                .await
                .map_err(|_| error())?,
        );
        if self.fail.load(Ordering::SeqCst) {
            return Err(error());
        }
        Ok(DestroyClassification::Applied)
    }
}
fn error() -> TemplateOutcomeError {
    TemplateOutcomeError::ExecutionFailed {
        phase: "injected".into(),
    }
}
fn provenance(id: &str) -> PlanProvenance {
    PlanProvenance {
        intent: shaula_core::plan::PlanIntent::Create,
        saved_plan_digest: "sha256:plan".into(),
        engine_kind: "terraform".into(),
        engine_version: "1.9.8".into(),
        engine_binary_digest: "sha256:engine".into(),
        artifact_digest: "sha256:artifact".into(),
        template_material_digest: "sha256:material".into(),
        protected_input_digest: "sha256:input".into(),
        state_lineage: StateLineage::Empty,
        generation_id: id.into(),
        attempt_id: format!("create-{id}"),
    }
}

pub struct Fixture {
    pub store: Arc<SqliteControlPlane>,
    pub github: Arc<fakes::GitHub>,
    pub runtime: Arc<Runtime>,
    pub clock: Arc<Clock>,
    pub digest: String,
    root: std::path::PathBuf,
    gates: Arc<shaula_daemon::effect_gate::FleetEffectGates>,
}
impl Fixture {
    pub async fn new() -> TestResult<Self> {
        let (app, store, engine, root) = build_app_with_artifact_root().await;
        let digest = attestation_harness::seed_profile(&app, &store, "k8s-linux", true).await;
        let body = attest_body(
            &store,
            &digest,
            &expected_bindings_digest("k8s-linux", 1),
            &engine,
        )
        .await;
        assert_eq!(
            attestation_harness::put_attestation(&app, "k8s-linux", 1, "att", body)
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
            .await?
            .status(),
            axum::http::StatusCode::ACCEPTED
        );
        let clock = Arc::new(Clock(AtomicI64::new(1000)));
        Ok(Self {
            store,
            root,
            digest,
            clock: clock.clone(),
            github: Arc::new(fakes::GitHub::default()),
            runtime: Arc::new(Runtime {
                destroys: AtomicUsize::new(0),
                fail: AtomicBool::new(false),
                clock,
            }),
            gates: Arc::new(shaula_daemon::effect_gate::FleetEffectGates::new()),
        })
    }
    pub fn supervisor(&self, limit: Duration) -> TestResult<FleetSupervisor> {
        Ok(FleetSupervisor::new(
            FleetSupervisorDeps {
                limits: LifecycleLimits {
                    create: Arc::new(tokio::sync::Semaphore::new(1)),
                    destroy: Arc::new(tokio::sync::Semaphore::new(1)),
                },
                store: self.store.clone(),
                handoff: self.store.clone(),
                github: self.github.clone(),
                runtime: self.runtime.clone(),
            },
            FleetSupervisorConfig {
                fleet_key: "f1".into(),
                capacity: shaula_core::capacity::CapacityPolicy {
                    min_runners: 0,
                    max_runners: 5,
                },
                work_root: self.root.join("work"),
                artifact_root: self.root.clone(),
                operation_timeout: Duration::from_secs(5),
                apply_intent_sink: Arc::new(LedgerApplyIntentSink {
                    store: self.store.clone(),
                    gates: self.gates.clone(),
                }),
                labels: vec![],
                auth_profile_key: "prod-app".into(),
                auth_revision: 1,
            },
            shaula_core::github::ScaleSetIdentity {
                target: shaula_core::github::GitHubTarget::organization("example-org")
                    .map_err(|e| format!("{e:?}"))?,
                runner_group: "Default".into(),
                scale_set_name: "shaula-x64".into(),
            },
        )
        .with_clock(self.clock.clone())
        .with_runner_max_lifetime(limit))
    }
    pub async fn seed(&self, proof: bool) -> TestResult {
        self.store
            .generation_insert(GenerationRecord {
                id: "gen".into(),
                fleet_key: "f1".into(),
                runner_name: "runner".into(),
                generation_name: "generation".into(),
                fleet_revision: 1,
                pool_member_key: None,
                template_profile_key: "k8s-linux".into(),
                template_revision: 1,
                template_artifact_digest: self.digest.clone(),
                attestation_id: "att".into(),
                inputs_digest: "inputs".into(),
                state: G::CreatePending,
                github_runner_id: None,
                workspace_path: self.root.join("work").to_string_lossy().into(),
                created_at: 1,
                updated_at: 1,
            })
            .await?;
        self.store
            .generation_set_github_runner("gen", 12, 1)
            .await?;
        for state in [G::Creating, G::WaitingOnline, G::Idle, G::Busy] {
            self.store.generation_advance("gen", state, 2).await?;
        }
        if proof {
            self.store
                .operation_record_apply_starting(&provenance("gen"), "plan", 2)
                .await?;
        }
        self.store
            .generation_set_result(
                "gen",
                r#"{"state_lineage":"lineage","state_serial":1}"#,
                "result",
                1000,
            )
            .await?;
        self.github
            .runners
            .lock()
            .map_err(|_| "poisoned")?
            .push(RunnerRef {
                id: 12,
                name: "runner".into(),
                scale_set_id: 42,
                status: "online".into(),
            });
        Ok(())
    }
    pub async fn state(&self) -> TestResult<G> {
        Ok(self
            .store
            .generation_get("gen")
            .await?
            .ok_or("missing")?
            .state)
    }
}
