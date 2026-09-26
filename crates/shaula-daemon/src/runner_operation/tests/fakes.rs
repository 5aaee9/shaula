//! Remote-API mocks for the two Runner Backend adapters and the IaC runtime.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use shaula_core::github::{GitHubTarget, Label, ScaleSetIdentity};
use shaula_core::ports::forgejo::{
    ForgejoAuthProbe, ForgejoJob, ForgejoPoolPort, ForgejoRegistration,
    ForgejoRegistrationUncertainty, ForgejoRemovalOutcome, ForgejoRunnerRef,
    ForgejoTemplateEvidence,
};
use shaula_core::ports::{
    AccessFailure, AuthContext, DestroyClassification, EffectOutcome, GitHubAccessPort, JitConfig,
    LookupOutcome, PollOutcome, RemovalOutcome, RouteProof, RunnerLookup, RunnerRef, ScaleSetView,
    SessionHandle, TargetIdentity, TemplateCreateRequest, TemplateCreateResult,
    TemplateDestroyRequest, TemplateOutcomeError, TemplateRuntimePort,
};

fn unavailable<T>() -> Result<T, AccessFailure> {
    Err(AccessFailure::Unavailable {
        summary: "not scripted".into(),
    })
}

/// Scripted answer of a GitHub remote call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Scripted {
    Idle,
    Busy,
    Unavailable,
    NameTaken,
}

pub(super) struct Github {
    pub script: Mutex<Scripted>,
    pub removals: AtomicUsize,
}

impl Default for Github {
    fn default() -> Self {
        Self {
            script: Mutex::new(Scripted::Idle),
            removals: AtomicUsize::new(0),
        }
    }
}

impl Github {
    fn script(&self) -> Scripted {
        self.script
            .lock()
            .map_or(Scripted::Unavailable, |script| *script)
    }
}

#[async_trait::async_trait]
impl GitHubAccessPort for Github {
    async fn auth_context(&self) -> AuthContext {
        AuthContext {
            kind: shaula_core::auth::AuthKind::GithubApp,
        }
    }
    async fn resolve_runner_group(&self, _: &ScaleSetIdentity) -> Result<i64, AccessFailure> {
        unavailable()
    }
    async fn lookup_scale_set(
        &self,
        _: &ScaleSetIdentity,
        _: i64,
    ) -> Result<LookupOutcome, AccessFailure> {
        unavailable()
    }
    async fn create_scale_set(
        &self,
        _: &ScaleSetIdentity,
        _: i64,
        _: &[Label],
    ) -> Result<EffectOutcome<ScaleSetView>, AccessFailure> {
        unavailable()
    }
    async fn establish_session(
        &self,
        _: i64,
        _: &str,
    ) -> Result<EffectOutcome<SessionHandle>, AccessFailure> {
        unavailable()
    }
    async fn delete_session(&self, _: i64, _: &str) -> Result<(), AccessFailure> {
        unavailable()
    }
    async fn poll_messages(
        &self,
        _: &SessionHandle,
        _: i64,
        _: i64,
    ) -> Result<PollOutcome, AccessFailure> {
        unavailable()
    }
    async fn ack_message(&self, _: &SessionHandle, _: i64) -> Result<(), AccessFailure> {
        unavailable()
    }
    async fn acquire_jobs(
        &self,
        _: i64,
        _: &SessionHandle,
        _: &[i64],
    ) -> Result<EffectOutcome<Vec<i64>>, AccessFailure> {
        unavailable()
    }
    async fn generate_jit(
        &self,
        _: i64,
        _: &str,
    ) -> Result<EffectOutcome<JitConfig>, AccessFailure> {
        unavailable()
    }
    async fn get_runner_by_name(&self, _: i64, name: &str) -> Result<RunnerLookup, AccessFailure> {
        match self.script() {
            Scripted::NameTaken => Ok(RunnerLookup::ExactlyOne(RunnerRef {
                id: 99,
                name: name.into(),
                scale_set_id: 42,
                status: "online".into(),
            })),
            Scripted::Unavailable => unavailable(),
            Scripted::Idle | Scripted::Busy => Ok(RunnerLookup::None),
        }
    }
    async fn remove_runner(&self, _: i64) -> Result<RemovalOutcome, AccessFailure> {
        self.removals.fetch_add(1, Ordering::SeqCst);
        match self.script() {
            Scripted::Busy => Ok(RemovalOutcome::JobStillRunning),
            Scripted::Unavailable => unavailable(),
            Scripted::Idle | Scripted::NameTaken => Ok(RemovalOutcome::Removed),
        }
    }
    async fn list_runners(&self, _: i64) -> Result<Vec<RunnerRef>, AccessFailure> {
        unavailable()
    }
    fn allows_target(&self, _: &GitHubTarget) -> bool {
        true
    }
    async fn resolve_target_identity(
        &self,
        _: &GitHubTarget,
    ) -> Result<TargetIdentity, AccessFailure> {
        unavailable()
    }
    async fn ensure_route_proof(&self) -> Result<RouteProof, AccessFailure> {
        unavailable()
    }
}

#[derive(Default)]
pub(super) struct Forgejo {
    pub runners: Mutex<Vec<ForgejoRunnerRef>>,
    pub fail_inventory: AtomicBool,
    pub deletes: AtomicUsize,
}

impl Forgejo {
    fn runners(&self) -> Result<Vec<ForgejoRunnerRef>, AccessFailure> {
        if self.fail_inventory.load(Ordering::SeqCst) {
            return unavailable();
        }
        self.runners
            .lock()
            .map_or_else(|_| unavailable(), |runners| Ok(runners.clone()))
    }
}

#[async_trait::async_trait]
impl ForgejoPoolPort for Forgejo {
    async fn register_runner(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<EffectOutcome<ForgejoRegistration>, AccessFailure> {
        unavailable()
    }
    async fn list_runners(&self) -> Result<Vec<ForgejoRunnerRef>, AccessFailure> {
        self.runners()
    }
    async fn get_runner(&self, id: u64) -> Result<Option<ForgejoRunnerRef>, AccessFailure> {
        Ok(self.runners()?.into_iter().find(|runner| runner.id == id))
    }
    async fn list_jobs(&self, _: &[String]) -> Result<Vec<ForgejoJob>, AccessFailure> {
        unavailable()
    }
    async fn task_results(
        &self,
        _: u64,
        _: &[u64],
    ) -> Result<Vec<shaula_core::jobs::ForgejoTaskResult>, AccessFailure> {
        unavailable()
    }
    async fn delete_runner(&self, id: u64) -> Result<ForgejoRemovalOutcome, AccessFailure> {
        self.deletes.fetch_add(1, Ordering::SeqCst);
        let mut runners = self
            .runners
            .lock()
            .map_err(|_| AccessFailure::Unavailable {
                summary: "poisoned".into(),
            })?;
        runners.retain(|runner| runner.id != id);
        Ok(ForgejoRemovalOutcome::Removed)
    }
    async fn classify_uncertain_registration(
        &self,
        _: &str,
        _: &[String],
    ) -> Result<ForgejoRegistrationUncertainty, AccessFailure> {
        unavailable()
    }
    async fn auth_probe(&self) -> Result<ForgejoAuthProbe, AccessFailure> {
        unavailable()
    }
}

pub(super) struct Runtime {
    pub destroy_ok: AtomicBool,
    pub destroys: AtomicUsize,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            destroy_ok: AtomicBool::new(true),
            destroys: AtomicUsize::new(0),
        }
    }
}

#[async_trait::async_trait]
impl TemplateRuntimePort for Runtime {
    async fn forgejo_template_evidence(&self, _: &str) -> ForgejoTemplateEvidence {
        ForgejoTemplateEvidence::ProcessIdle
    }
    async fn create(
        &self,
        _: TemplateCreateRequest,
    ) -> Result<TemplateCreateResult, TemplateOutcomeError> {
        Err(TemplateOutcomeError::PlanFailed {
            phase: "not scripted".into(),
        })
    }
    async fn destroy(
        &self,
        _: TemplateDestroyRequest,
    ) -> Result<DestroyClassification, TemplateOutcomeError> {
        self.destroys.fetch_add(1, Ordering::SeqCst);
        if self.destroy_ok.load(Ordering::SeqCst) {
            Ok(DestroyClassification::Applied)
        } else {
            Err(TemplateOutcomeError::ExecutionFailed {
                phase: "scripted destroy failure".into(),
            })
        }
    }
}
