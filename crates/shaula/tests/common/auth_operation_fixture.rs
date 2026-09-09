use super::common::*;
use shaula_core::{plan::PlanIntent, ports::*, template::ShaulaResultEnvelope};
use shaula_daemon::supervisor::*;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use tower::ServiceExt;

#[path = "lifecycle_fakes.rs"]
pub mod fakes;

#[derive(Default)]
pub struct Runtime {
    pub create_uncertain: AtomicBool,
    pub destroy_uncertain_once: AtomicBool,
    pub creates: AtomicUsize,
    pub destroys: AtomicUsize,
}

#[async_trait::async_trait]
impl TemplateRuntimePort for Runtime {
    async fn create(
        &self,
        request: TemplateCreateRequest,
    ) -> Result<TemplateCreateResult, TemplateOutcomeError> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        let provenance = PlanProvenance {
            intent: PlanIntent::Create,
            saved_plan_digest: "sha256:create-plan".into(),
            engine_kind: "terraform".into(),
            engine_version: "1.9.0".into(),
            engine_binary_digest: "sha256:engine".into(),
            artifact_digest: request.pinned_artifact_digest,
            template_material_digest: "sha256:material".into(),
            protected_input_digest: "sha256:input".into(),
            state_lineage: StateLineage::Empty,
            generation_id: request.input.generation.id.clone(),
            attempt_id: shaula_core::auth::new_attempt_id(),
        };
        let claim = request
            .apply_intent_sink
            .as_ref()
            .unwrap()
            .persist_apply_starting(&provenance)
            .await
            .unwrap();
        drop(claim);
        if self.create_uncertain.load(Ordering::SeqCst) {
            return Err(TemplateOutcomeError::ExecutionFailed {
                phase: "create.apply".into(),
            });
        }
        Ok(TemplateCreateResult {
            result_envelope: ShaulaResultEnvelope {
                contract_version: 1,
                generation_id: request.input.generation.id,
                bindings_digest: request.expected_bindings_digest,
                resources: vec![shaula_core::template::ResultResource {
                    role: "runner".into(),
                    id: "fixture-resource".into(),
                    incarnation: None,
                }],
            },
            state_lineage: "fixture-lineage".into(),
            state_serial: 1,
            provenance,
        })
    }

    async fn destroy(
        &self,
        request: TemplateDestroyRequest,
    ) -> Result<DestroyClassification, TemplateOutcomeError> {
        self.destroys.fetch_add(1, Ordering::SeqCst);
        let mut provenance = request.original_provenance;
        provenance.intent = PlanIntent::Destroy;
        provenance.attempt_id = shaula_core::auth::new_attempt_id();
        provenance.saved_plan_digest = "sha256:destroy-plan".into();
        provenance.state_lineage = StateLineage::Serial {
            lineage: request.original_state.lineage,
            serial: request.original_state.serial,
        };
        let claim = request
            .apply_intent_sink
            .as_ref()
            .unwrap()
            .persist_apply_starting(&provenance)
            .await
            .unwrap();
        drop(claim);
        if self.destroy_uncertain_once.swap(false, Ordering::SeqCst) {
            return Err(TemplateOutcomeError::ExecutionFailed {
                phase: "destroy.apply".into(),
            });
        }
        Ok(DestroyClassification::Applied)
    }
}

pub struct Harness {
    pub app: axum::Router,
    pub store: Arc<shaula_store::registry_impl::SqliteControlPlane>,
    pub github: Arc<fakes::GitHub>,
    pub runtime: Arc<Runtime>,
    root: std::path::PathBuf,
}

impl Harness {
    pub async fn new() -> Self {
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
            app.clone()
                .oneshot(authorized(
                    "PUT",
                    "/api/v1/fleets/f1",
                    Some(FLEET_BODY.into())
                ))
                .await
                .unwrap()
                .status(),
            axum::http::StatusCode::ACCEPTED
        );
        Self {
            app,
            store,
            github: Arc::default(),
            runtime: Arc::default(),
            root,
        }
    }

    pub fn supervisor(&self, revision: i64) -> FleetSupervisor {
        let supervisor = FleetSupervisor::new(
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
                    max_runners: 1,
                },
                work_root: self.root.join("work"),
                artifact_root: self.root.clone(),
                operation_timeout: std::time::Duration::from_secs(1),
                apply_intent_sink: Arc::new(shaula_daemon::apply_intent::LedgerApplyIntentSink {
                    store: self.store.clone(),
                    gates: Arc::default(),
                }),
                labels: vec![shaula_core::github::Label {
                    name: "shaula-x64".into(),
                    label_type: "Customer".into(),
                }],
                auth_profile_key: "prod-app".into(),
                auth_revision: revision,
            },
            shaula_core::github::ScaleSetIdentity {
                target: shaula_core::github::GitHubTarget::organization("example-org").unwrap(),
                runner_group: "Default".into(),
                scale_set_name: "shaula-x64".into(),
            },
        );
        supervisor.with_revision_clients(std::collections::HashMap::from([(
            ("prod-app".into(), 1),
            self.github.clone() as Arc<dyn GitHubAccessPort>,
        )]))
    }

    pub async fn rotate(&self) -> FleetSupervisor {
        let path = "/api/v1/github-auth-profiles/prod-app";
        let current = self
            .app
            .clone()
            .oneshot(authorized("GET", path, None))
            .await
            .unwrap();
        let etag = current.headers().get("etag").unwrap().clone();
        let mut put = authorized(
            "PUT",
            path,
            Some(AUTH_PUT_BODY.replace("test_key_bytes", "rotated_token_bytes")),
        );
        put.headers_mut().remove("if-none-match");
        put.headers_mut().insert("if-match", etag);
        assert_eq!(
            self.app.clone().oneshot(put).await.unwrap().status(),
            axum::http::StatusCode::ACCEPTED
        );
        crate::common::auth_fixture::promote(self.store.as_ref(), "prod-app", 2, 25)
            .await
            .unwrap();
        let supervisor = self.supervisor(2);
        assert!(supervisor.tick(30).await.unwrap().handoff_acknowledged);
        supervisor
    }
}
