use super::*;
pub(super) async fn setup() -> (
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

pub(super) async fn setup_with_labels(
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

pub(super) async fn seed_idle(store: &impl LifecycleStore, digest: &str) {
    store
        .generation_insert(GenerationRecord {
            id: "gen1".into(),
            fleet_key: "f1".into(),
            runner_name: "runner1".into(),
            generation_name: "generation1".into(),
            fleet_revision: 1,
            pool_member_key: None,
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
