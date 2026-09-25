//! One Fleet over a real SQLite ledger, parameterized by Runner Backend. The
//! store is a local stand-in; only the remote APIs and the IaC runtime are
//! mocked.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use shaula_core::lifecycle::GenerationState as G;
use shaula_core::ports::forgejo::ForgejoRunnerRef;
use shaula_core::ports::{ApplyIntentSink, GitHubAccessPort};
use shaula_core::registry::{
    ControlPlaneStore, FleetRuntimeGuard, GenerationRecord, LifecycleStore, ScaleSetRow,
};
use shaula_store::registry_impl::SqliteControlPlane;
use shaula_store::Store;

use super::fakes::{Forgejo, Github, Runtime, Scripted};
use crate::effect_gate::FleetEffectGates;
use crate::runner_operation::forgejo::ForgejoRegistrations;
use crate::runner_operation::github::GithubRegistrations;
use crate::runner_operation::{Registration, RemovalGate, RunnerOperation, RunnerRegistrations};

pub(super) type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

pub(super) const FLEET: &str = "fleet";
pub(super) const GENERATION: &str = "generation";
pub(super) const RUNNER_NAME: &str = "pool-generation";
pub(super) const RUNNER_ID: i64 = 55;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Backend {
    Github,
    Forgejo,
}

pub(super) const BACKENDS: [Backend; 2] = [Backend::Github, Backend::Forgejo];

/// The test-side Runner Backend adapter: one of the two production adapters.
pub(super) enum Adapter<'a> {
    Github(GithubRegistrations<'a>),
    Forgejo(ForgejoRegistrations<'a>),
}

impl RunnerRegistrations for Adapter<'_> {
    async fn admit_removal(
        &self,
        generation: &GenerationRecord,
        create_started: bool,
    ) -> shaula_core::error::CoreResult<Registration> {
        match self {
            Self::Github(adapter) => adapter.admit_removal(generation, create_started).await,
            Self::Forgejo(adapter) => adapter.admit_removal(generation, create_started).await,
        }
    }

    async fn remove(
        &self,
        generation: &GenerationRecord,
        gate: RemovalGate,
    ) -> shaula_core::error::CoreResult<Registration> {
        match self {
            Self::Github(adapter) => adapter.remove(generation, gate).await,
            Self::Forgejo(adapter) => adapter.remove(generation, gate).await,
        }
    }

    async fn prove_unrecorded_absent(
        &self,
        generation: &GenerationRecord,
    ) -> shaula_core::error::CoreResult<Registration> {
        match self {
            Self::Github(adapter) => adapter.prove_unrecorded_absent(generation).await,
            Self::Forgejo(adapter) => adapter.prove_unrecorded_absent(generation).await,
        }
    }
}

pub(super) struct Fixture {
    pub backend: Backend,
    _directory: tempfile::TempDir,
    pub artifact_root: std::path::PathBuf,
    pub store: Arc<SqliteControlPlane>,
    pub github: Arc<Github>,
    github_port: Arc<dyn GitHubAccessPort>,
    pub forgejo: Arc<Forgejo>,
    pub runtime: Arc<Runtime>,
    gates: FleetEffectGates,
    guard: FleetRuntimeGuard,
    apply_intent: Arc<dyn ApplyIntentSink>,
    revision_clients: HashMap<(String, i64), Arc<dyn GitHubAccessPort>>,
    labels: Vec<String>,
}

pub(super) fn digest() -> String {
    format!("sha256:{}", "a".repeat(64))
}

impl Fixture {
    pub async fn new(backend: Backend) -> TestResult<Self> {
        let directory = tempfile::tempdir()?;
        let db = Store::open(&directory.path().join("state.db")).await?;
        db.migrate().await?;
        let digest = digest();
        let (auth, spec_json) = match backend {
            Backend::Github => ("github", "{}"),
            Backend::Forgejo => (
                "forgejo",
                r#"{"kind":"forgejo","forgejo":{"instance_url":"https://forgejo.test",
                "scope":{"kind":"instance"},"auth_profile_ref":"forgejo",
                "runner_name_prefix":"pool-","labels":["linux:host"]},
                "capacity":{"min_runners":0,"max_runners":2},"template_profile_ref":"profile"}"#,
            ),
        };
        db.execute_for_tests(
            "INSERT INTO fleets (key,incarnation,desired_revision,observed_revision,mutation_fence,
             deletion_marker,phase,tombstone,created_at,updated_at)
             VALUES ('fleet','inc',1,0,1,0,'Pending',0,1,1)",
        )
        .await?;
        db.execute_for_tests(&format!(
            "INSERT INTO fleet_revisions (fleet_key,incarnation,revision,spec_json,template_profile_key,
             template_revision,template_artifact_digest,template_attestation_id,auth_desired_profile_key,
             auth_desired_revision,inputs_digest,created_at)
             VALUES ('fleet','inc',1,'{spec_json}','profile',1,'{digest}','attestation','{auth}',1,'inputs',1)"
        ))
        .await?;
        db.execute_for_tests(&format!(
            "INSERT INTO template_profile_revisions (profile_key,revision,artifact_digest,engine_ref,
             bindings_json,bindings_digest,state,created_at)
             VALUES ('profile',1,'{digest}','terraform','{{}}','bindings','Active',1)"
        ))
        .await?;
        let artifact_root = directory.path().join("artifacts");
        let artifact = shaula_core::artifact_layout::artifact_dir(&artifact_root, &digest)
            .ok_or("bad digest")?;
        std::fs::create_dir_all(&artifact)?;
        std::fs::copy(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../templates/docker/profile.yaml"),
            artifact.join("profile.yaml"),
        )?;
        let store = Arc::new(SqliteControlPlane::new(db, artifact_root.clone()));
        store
            .scale_set_upsert(ScaleSetRow {
                fleet_key: FLEET.into(),
                scale_set_id: Some(42),
                owned_scale_set_id: None,
                name: "scale-set".into(),
                runner_group: "default".into(),
                fingerprint: "sha256:fingerprint".into(),
                state: "Bound".into(),
                attempt_id: None,
                now: 1,
            })
            .await?;
        let head = store.fleet_get(FLEET).await?.ok_or("missing head")?;
        let github = Arc::new(Github::default());
        let gates = FleetEffectGates::new();
        let apply_intent: Arc<dyn ApplyIntentSink> =
            Arc::new(crate::apply_intent::LedgerApplyIntentSink {
                store: store.clone(),
                gates: Arc::new(FleetEffectGates::new()),
            });
        Ok(Self {
            backend,
            _directory: directory,
            artifact_root,
            store,
            github_port: github.clone(),
            github,
            forgejo: Arc::new(Forgejo::default()),
            runtime: Arc::new(Runtime::default()),
            gates,
            guard: FleetRuntimeGuard::from(&head),
            apply_intent,
            revision_clients: HashMap::new(),
            labels: vec!["linux".into()],
        })
    }

    pub fn operation(&self) -> RunnerOperation<'_, Adapter<'_>> {
        let registrations = match self.backend {
            Backend::Github => Adapter::Github(GithubRegistrations {
                fleet_key: FLEET,
                lifecycle: self.store.as_ref(),
                store: self.store.as_ref(),
                current: ("github", 1),
                github: &self.github_port,
                revision_clients: &self.revision_clients,
            }),
            Backend::Forgejo => Adapter::Forgejo(ForgejoRegistrations {
                fleet_key: FLEET,
                guard: &self.guard,
                lifecycle: self.store.as_ref(),
                store: self.store.as_ref(),
                gates: &self.gates,
                forgejo: self.forgejo.as_ref(),
                runtime: self.runtime.as_ref(),
                labels: &self.labels,
            }),
        };
        RunnerOperation {
            lifecycle: self.store.as_ref(),
            store: self.store.as_ref(),
            runtime: self.runtime.as_ref(),
            clock: None,
            artifact_root: &self.artifact_root,
            operation_timeout: std::time::Duration::from_secs(5),
            apply_intent: &self.apply_intent,
            provider: match self.backend {
                Backend::Github => shaula_core::fleet::FleetProviderKind::Github,
                Backend::Forgejo => shaula_core::fleet::FleetProviderKind::Forgejo,
            },
            registrations,
        }
    }

    /// Scripts the remote registration: the runner exists with this status.
    pub fn remote(&self, remote: Scripted) -> TestResult {
        *self.github.script.lock().map_err(|_| "script poisoned")? = remote;
        let status = match remote {
            Scripted::Busy => "active",
            _ => "idle",
        };
        let (id, name) = match remote {
            // A foreign runner that took the Generation's exact name.
            Scripted::NameTaken => (99, RUNNER_NAME),
            _ => (u64::try_from(RUNNER_ID)?, RUNNER_NAME),
        };
        *self
            .forgejo
            .runners
            .lock()
            .map_err(|_| "runners poisoned")? = vec![ForgejoRunnerRef {
            id,
            uuid: if remote == Scripted::NameTaken {
                "foreign".into()
            } else {
                "uuid-55".into()
            },
            name: name.into(),
            status: status.into(),
            labels: self.labels.clone(),
            ephemeral: true,
            version: Some("13".into()),
        }];
        self.forgejo
            .fail_inventory
            .store(remote == Scripted::Unavailable, Ordering::SeqCst);
        Ok(())
    }

    /// No runner with the Generation's name exists remotely.
    pub fn remote_absent(&self) -> TestResult {
        self.remote(Scripted::Idle)?;
        self.forgejo
            .runners
            .lock()
            .map_err(|_| "runners poisoned")?
            .clear();
        Ok(())
    }

    pub fn corrupt_manifest(&self) -> TestResult {
        let artifact = shaula_core::artifact_layout::artifact_dir(&self.artifact_root, &digest())
            .ok_or("bad digest")?;
        std::fs::write(artifact.join("profile.yaml"), "api_version: [unterminated")?;
        Ok(())
    }

    pub async fn generation(&self) -> TestResult<GenerationRecord> {
        let lifecycle: &dyn LifecycleStore = self.store.as_ref();
        Ok(lifecycle
            .generation_get(GENERATION)
            .await?
            .ok_or("missing generation")?)
    }

    pub async fn state(&self) -> TestResult<G> {
        Ok(self.generation().await?.state)
    }

    pub fn destroys(&self) -> usize {
        self.runtime.destroys.load(Ordering::SeqCst)
    }

    /// Remote registration removals attempted through the backend API.
    pub fn removals(&self) -> usize {
        match self.backend {
            Backend::Github => self.github.removals.load(Ordering::SeqCst),
            Backend::Forgejo => self.forgejo.deletes.load(Ordering::SeqCst),
        }
    }
}
