use super::forgejo_pool_fakes::{Forgejo, Runtime};
use crate::{registry_impl::SqliteControlPlane, Store};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use shaula_core::{
    capacity::CapacityPolicy,
    fleet::FleetSpec,
    lifecycle::GenerationState as G,
    registry::{ControlPlaneStore, FleetRuntimeGuard, GenerationRecord, LifecycleStore},
};
use shaula_daemon::{
    effect_gate::FleetEffectGates,
    forgejo_supervisor::{ForgejoPoolSupervisor, ForgejoPoolSupervisorDeps},
};
use std::sync::{
    atomic::{AtomicI64, Ordering},
    Arc,
};

pub(super) type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
pub(super) struct Clock(pub AtomicI64);
impl shaula_core::ports::Clock for Clock {
    fn now_unix_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

pub(super) struct Fixture {
    pub directory: tempfile::TempDir,
    pub store: Arc<SqliteControlPlane>,
    pub forgejo: Arc<Forgejo>,
    pub runtime: Arc<Runtime>,
    pub clock: Arc<Clock>,
    pub gates: Arc<FleetEffectGates>,
    pub spec: FleetSpec,
}
impl Fixture {
    pub async fn new(min: i64, max: i64) -> TestResult<Self> {
        let directory = tempfile::tempdir()?;
        let db = Store::open(&directory.path().join("state.db")).await?;
        db.migrate().await?;
        let digest = format!("sha256:{}", "a".repeat(64));
        let spec_json = serde_json::json!({
            "kind": "forgejo", "forgejo": {
                "instance_url": "https://forgejo.test", "scope": {"kind":"instance"},
                "auth_profile_ref":"forgejo", "runner_name_prefix":"pool-", "labels":["linux:host"]
            },
            "capacity": {"min_runners":min,"max_runners":max},
            "template_profile_ref": "profile"
        })
        .to_string();
        let spec = serde_json::from_str(&spec_json)?;
        db.connection().execute_unprepared("INSERT INTO fleets
            (key,incarnation,desired_revision,observed_revision,mutation_fence,deletion_marker,phase,tombstone,created_at,updated_at)
            VALUES ('fleet','inc',1,0,1,0,'Pending',0,1,1)").await?;
        db.connection().execute(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
            "INSERT INTO fleet_revisions (fleet_key,incarnation,revision,spec_json,template_profile_key,template_revision,
            template_artifact_digest,template_attestation_id,auth_desired_profile_key,auth_desired_revision,inputs_digest,created_at)
            VALUES ('fleet','inc',1,?,'profile',1,?,'attestation','forgejo',1,'inputs',1)",
            [spec_json.into(),digest.clone().into()])).await?;
        db.connection().execute(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
            "INSERT INTO template_profile_revisions (profile_key,revision,artifact_digest,engine_ref,bindings_json,bindings_digest,state,created_at)
            VALUES ('profile',1,?,'terraform','{}','bindings','Active',1)", [digest.clone().into()])).await?;
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../templates/docker/profile.yaml");
        let mut manifest: shaula_core::template::ProfileManifest =
            serde_yaml::from_str(&std::fs::read_to_string(source)?)?;
        manifest.runner_backend = "forgejo".into();
        manifest.runner_image_digests = vec![format!(
            "code.forgejo.org/forgejo/runner:13.1.0@sha256:{}",
            "b".repeat(64)
        )];
        let artifact_root = directory.path().join("artifacts");
        let artifact = shaula_core::artifact_layout::artifact_dir(&artifact_root, &digest)
            .ok_or("bad digest")?;
        std::fs::create_dir_all(&artifact)?;
        std::fs::write(
            artifact.join("profile.yaml"),
            serde_yaml::to_string(&manifest)?,
        )?;
        Ok(Self {
            directory,
            store: Arc::new(SqliteControlPlane::new(db, artifact_root)),
            forgejo: Arc::new(Forgejo::default()),
            runtime: Arc::new(Runtime::default()),
            clock: Arc::new(Clock(AtomicI64::new(1_000))),
            gates: Arc::new(FleetEffectGates::new()),
            spec,
        })
    }
    pub async fn supervisor(&self) -> TestResult<ForgejoPoolSupervisor> {
        let head = self.store.fleet_get("fleet").await?.ok_or("missing head")?;
        Ok(ForgejoPoolSupervisor::new(
            "fleet",
            self.spec
                .forgejo
                .as_ref()
                .ok_or("missing Forgejo section")?,
            CapacityPolicy {
                min_runners: self.spec.capacity.min_runners,
                max_runners: self.spec.capacity.max_runners,
            },
            ForgejoPoolSupervisorDeps {
                guard: FleetRuntimeGuard::from(&head),
                auth: ("forgejo".into(), 1),
                store: self.store.clone(),
                lifecycle: self.store.clone(),
                forgejo: self.forgejo.clone(),
                clock: self.clock.clone(),
                runtime: self.runtime.clone(),
                gates: self.gates.clone(),
                apply_intent_sink: Arc::new(shaula_daemon::apply_intent::LedgerApplyIntentSink {
                    store: self.store.clone(),
                    gates: self.gates.clone(),
                }),
                create_limit: Arc::new(tokio::sync::Semaphore::new(1)),
                destroy_limit: Arc::new(tokio::sync::Semaphore::new(1)),
                work_root: self.directory.path().join("work"),
                artifact_root: self.directory.path().join("artifacts"),
                operation_timeout: std::time::Duration::from_secs(5),
                setup_info_issuer: None,
            },
        )?)
    }
    pub async fn generation(&self) -> TestResult<GenerationRecord> {
        self.store
            .generations_for_fleet("fleet")
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| "missing generation".into())
    }
    pub async fn seed(&self, state: G) -> TestResult<GenerationRecord> {
        let record = GenerationRecord {
            id: "generation".into(),
            fleet_key: "fleet".into(),
            runner_name: "pool-generation".into(),
            generation_name: "generation".into(),
            fleet_revision: 1,
            template_profile_key: "profile".into(),
            template_revision: 1,
            template_artifact_digest: format!("sha256:{}", "a".repeat(64)),
            attestation_id: "attestation".into(),
            inputs_digest: "inputs".into(),
            state: G::CreatePending,
            github_runner_id: None,
            workspace_path: self.directory.path().join("work").to_string_lossy().into(),
            created_at: 1,
            updated_at: 1,
        };
        self.store.generation_insert(record).await?;
        if state == G::CreatePending {
            return self.generation().await;
        }
        self.store
            .generation_advance("generation", G::Creating, 1)
            .await?;
        if state == G::Creating {
            return self.generation().await;
        }
        if matches!(state, G::Idle | G::Busy) {
            self.store
                .generation_advance("generation", G::WaitingOnline, 1)
                .await?;
            self.store
                .generation_advance("generation", G::Idle, 1)
                .await?;
            if state == G::Busy {
                self.store
                    .generation_advance("generation", G::Busy, 1)
                    .await?;
            }
        } else {
            self.store
                .generation_advance("generation", state, 1)
                .await?;
        }
        self.generation().await
    }
    pub fn declare(&self, status: &str) -> TestResult<()> {
        let mut runners = self
            .forgejo
            .runners
            .lock()
            .map_err(|_| "runners poisoned")?;
        for runner in runners.iter_mut() {
            runner.status = status.into();
            runner.labels = vec!["linux".into()];
        }
        Ok(())
    }
}
