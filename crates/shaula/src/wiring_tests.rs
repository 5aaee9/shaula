//! Composition tests for `SupervisorWiring` (declared as a child module
//! of `wiring`): the REAL scheduling loop drives the REAL v2 worker, and
//! the REAL execution-authority resolution binds each supervisor to the
//! persisted observed context. No test-only dispatch anywhere.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::SupervisorWiring;
use shaula_core::ports::TemplateRuntimePort;
use shaula_core::registry::{Actor, ChangeView, ControlPlaneStore, MutationFacts, Scope};
use shaula_store::registry_impl::SqliteControlPlane;
use shaula_store::Store;
use std::path::PathBuf;
use std::sync::Arc;

/// The composition Profile key, shared with the auth-worker test suite.
use crate::auth_worker_v2::tests::KEY;

// ---- Scheduling-loop support ------------------------------------------------

pub(crate) struct NoRuntime;
#[async_trait::async_trait]
impl TemplateRuntimePort for NoRuntime {
    async fn create(
        &self,
        _: shaula_core::ports::TemplateCreateRequest,
    ) -> Result<shaula_core::ports::TemplateCreateResult, shaula_core::ports::TemplateOutcomeError>
    {
        Err(shaula_core::ports::TemplateOutcomeError::PlanFailed {
            phase: "scheduling-test".into(),
        })
    }
    async fn destroy(
        &self,
        _: shaula_core::ports::TemplateDestroyRequest,
    ) -> Result<shaula_core::ports::DestroyClassification, shaula_core::ports::TemplateOutcomeError>
    {
        Err(shaula_core::ports::TemplateOutcomeError::PlanFailed {
            phase: "scheduling-test".into(),
        })
    }
}

pub(crate) async fn wiring_with(
    control_plane: &Arc<SqliteControlPlane>,
    endpoints: crate::auth_worker_probe::WorkerEndpoints,
) -> SupervisorWiring {
    let limits = shaula_daemon::supervisor::LifecycleLimits {
        create: Arc::new(tokio::sync::Semaphore::new(1)),
        destroy: Arc::new(tokio::sync::Semaphore::new(1)),
    };
    SupervisorWiring::new(
        control_plane.clone(),
        control_plane.clone(),
        Arc::new(NoRuntime),
        Arc::new(crate::auth_worker_mock::Now),
        Arc::new(shaula_daemon::effect_gate::FleetEffectGates::new()),
        std::path::PathBuf::from("target/unused-work"),
        std::path::PathBuf::from("target/unused-artifacts"),
        std::time::Duration::from_secs(1),
        limits,
    )
    .with_auth_worker_endpoints(endpoints)
}

/// Drives ONE scheduling pass and drains the spawned tasks.
pub(crate) async fn tick_and_drain(wiring: &mut SupervisorWiring, now: i64) {
    wiring.clock = Arc::new(At(now));
    wiring.tick_all(now).await.unwrap();
    wiring.drain_for_tests().await;
}

// ---- G3 support: a fleet whose execution authority is fully persisted ------

/// A real SQLite control plane that KEEPS its tempdir alive, plus the
/// database path for direct durable-state manipulation in corruption
/// scenarios.
pub(crate) struct TestPlane {
    pub control_plane: Arc<SqliteControlPlane>,
    pub db_path: PathBuf,
    _tmp: tempfile::TempDir,
}

pub(crate) async fn test_plane() -> TestPlane {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("wiring.db");
    let store = Store::open(&db_path).await.unwrap();
    store.migrate().await.unwrap();
    TestPlane {
        control_plane: Arc::new(SqliteControlPlane::new(store, tmp.path().to_path_buf())),
        db_path,
        _tmp: tmp,
    }
}

pub(crate) const FLEET: &str = "f1";
const REPO_TARGET_JSON: &str = r#"{"kind":"repository","owner":"5aaee9","repository":"proj"}"#;

/// Seeds the full durable chain through the REAL store APIs: a promoted
/// v2 revision with a frozen Account Binding, then a fleet commit that
/// derives the desired context and the handoff tuple in the same
/// transaction (spec 0011 §4.2).
pub(crate) async fn seed_promoted_profile_and_fleet(plane: &TestPlane) -> i64 {
    crate::auth_worker_v2::tests::seed_candidate(
        &plane.control_plane,
        1,
        r#"{"selectors":[{"kind":"repository","owner":"5aaee9","repository":"proj"}]}"#,
    )
    .await;
    use shaula_core::auth_context::AccountBinding;
    let mut snapshot: shaula_core::registry::AuthValidationSnapshot =
        serde_json::from_str(&crate::auth_worker_mock::promotion_from(KEY, 1, &[]).snapshot_json)
            .unwrap();
    snapshot
        .identities
        .push(shaula_core::registry::AuthIdentityProof {
            login: "5aaee9".into(),
            account_id: 220,
            installation_id: 22,
            repositories: vec![shaula_core::registry::AuthRepoProof {
                owner: "5aaee9".into(),
                repository: "proj".into(),
                repository_id: 700,
                owner_id: 220,
            }],
        });
    let promotion = shaula_core::registry::AuthPromotion {
        bindings: vec![AccountBinding {
            account_id: 220,
            account_kind: shaula_core::auth_policy::AccountKind::User,
            login: "5aaee9".into(),
            installation_id: 22,
            repository_selection: shaula_core::auth_context::RepositorySelection::Selected,
            validated_at_ms: 1,
        }],
        snapshot_json: serde_json::to_string(&snapshot).unwrap(),
    };
    let outcome = ControlPlaneStore::auth_apply_validation_v2(
        plane.control_plane.as_ref(),
        KEY,
        1,
        true,
        None,
        5,
        Some(promotion),
    )
    .await
    .unwrap();
    assert_eq!(
        outcome,
        shaula_core::registry::AuthPromotionOutcome::Promoted,
        "fixture: promotion must succeed"
    );

    let spec_json = format!(
        r#"{{"github":{{"target":{REPO_TARGET_JSON},"auth_profile_ref":"{KEY}","scale_set_name":"shaula-x64","runner_group":"Default","labels":["shaula-x64"]}},"capacity":{{"min_runners":0,"max_runners":5}},"template_profile_ref":{{"key":"k8s-linux","revision":1}}}}"#
    );
    let facts = MutationFacts {
        resource_kind: "fleet",
        resource_key: FLEET.into(),
        incarnation: "inc-f1".into(),
        revision: 1,
        spec_json,
        template: None,
        auth_desired: Some((KEY.to_string(), 1)),
        inputs_digest: "sha256:inputs".into(),
        actor: "op".into(),
        now: 6,
        change: ChangeView {
            id: "f1-put".into(),
            resource_kind: "fleet".into(),
            resource_key: FLEET.into(),
            revision: 1,
            kind: "Put".into(),
            state: "Pending".into(),
            reason: None,
        },
        outbox_topic: "fleet.reconcile".into(),
        outbox_payload: "{}".into(),
        idempotency: None,
    };
    ControlPlaneStore::commit_fleet_mutation(plane.control_plane.as_ref(), facts)
        .await
        .unwrap()
        .unwrap();

    // The fleet's CURRENT mutation fence is the ack precondition (G5).
    plane
        .control_plane
        .fleet_get(FLEET)
        .await
        .unwrap()
        .unwrap()
        .mutation_fence
}

/// Acknowledges the handoff with the exact verified context — the same
/// fields the desired intent froze, so the store CAS accepts it and
/// persists the observed ref AND observed context atomically.
pub(crate) async fn acknowledge_handoff(plane: &TestPlane, fence: i64) {
    let context = shaula_core::auth_context::ResolvedAuthContext {
        profile_key: KEY.into(),
        revision: 1,
        github_host: "github.com".into(),
        app_id: crate::auth_worker_v2::tests::APP_ID.into(),
        account_id: 220,
        account_kind: shaula_core::auth_policy::AccountKind::User,
        login: "5aaee9".into(),
        installation_id: 22,
        target: shaula_core::github::GitHubTarget::new_repository("5aaee9", "proj").unwrap(),
        organization_id: None,
        repository_id: Some(700),
        repository_owner_id: Some(220),
    };
    let ack = ControlPlaneStore::handoff_acknowledge(
        plane.control_plane.as_ref(),
        FLEET,
        KEY,
        1,
        Some(&serde_json::to_string(&context).unwrap()),
        &shaula_core::registry::AuthHandoffExpectation {
            mutation_fence: fence,
            desired_context_json: plane
                .control_plane
                .fleet_auth_context_get(FLEET)
                .await
                .unwrap()
                .unwrap()
                .desired_context_json,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        ack,
        shaula_core::registry::FleetContextAck::Acknowledged,
        "fixture: the verified first handoff must acknowledge"
    );
}

/// Overwrites the durable observed context with garbage — simulating the
/// crash/corruption state the wiring must fail closed on.
pub(crate) async fn corrupt_observed_context(db_path: &std::path::Path) {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    let url = format!(
        "sqlite://{}?mode=rw",
        db_path.to_string_lossy().replace('\\', "/")
    );
    let db = sea_orm::Database::connect(url).await.unwrap();
    db.execute(Statement::from_string(
        DatabaseBackend::Sqlite,
        format!(
            "UPDATE fleet_auth_contexts SET observed_context_json = '{{broken' WHERE fleet_key = '{FLEET}'"
        ),
    ))
    .await
    .unwrap();
    db.close().await.unwrap();
}

/// Seeds the wiring WITHOUT spawning anything (execution tests never
/// reach the network; `supervisor_for` only resolves the authority).
pub(crate) async fn execution_wiring(plane: &TestPlane) -> SupervisorWiring {
    wiring_with(
        &plane.control_plane,
        crate::auth_worker_probe::WorkerEndpoints::production(),
    )
    .await
}

pub(super) fn fleet_actor() -> Actor {
    Actor {
        name: "wiring-test".into(),
        scopes: vec![Scope::FleetRead, Scope::FleetWrite, Scope::AuthRead],
    }
}

struct At(i64);
impl shaula_core::ports::Clock for At {
    fn now_unix_ms(&self) -> i64 {
        self.0
    }
}

#[path = "wiring_execution_tests.rs"]
mod execution_tests;
