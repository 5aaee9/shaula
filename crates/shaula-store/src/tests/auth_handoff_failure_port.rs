//! Controlled remote failures; effects panic so the test exercises proof only.
use shaula_core::{github::*, ports::*};
use tokio::sync::Notify;

#[derive(Clone, Copy)]
pub enum FailureStage {
    Route,
    Ownership,
    Identity,
}

pub struct FailingGitHub {
    pub reached: Notify,
    pub release: Notify,
    pub stage: FailureStage,
}

impl FailingGitHub {
    pub fn new(stage: FailureStage) -> Self {
        Self {
            reached: Notify::new(),
            release: Notify::new(),
            stage,
        }
    }
    async fn fail(&self) -> AccessFailure {
        self.reached.notify_one();
        self.release.notified().await;
        AccessFailure::PermissionDenied
    }
}

#[async_trait::async_trait]
impl GitHubAccessPort for FailingGitHub {
    async fn auth_context(&self) -> AuthContext {
        AuthContext {
            kind: shaula_core::auth::AuthKind::GithubApp,
        }
    }
    async fn ensure_route_proof(&self) -> Result<RouteProof, AccessFailure> {
        if matches!(self.stage, FailureStage::Route) {
            return Err(self.fail().await);
        }
        Ok(RouteProof {
            checked_at_unix_ms: 10,
            valid_until_unix_ms: 60_010,
            installation_id: 11,
            account_id: 100,
            organization_id: Some(100),
            repository_id: None,
            repository_owner_id: None,
        })
    }
    async fn resolve_runner_group(&self, _: &ScaleSetIdentity) -> Result<i64, AccessFailure> {
        if matches!(self.stage, FailureStage::Ownership) {
            return Err(self.fail().await);
        }
        Ok(7)
    }
    async fn lookup_scale_set(
        &self,
        _: &ScaleSetIdentity,
        _: i64,
    ) -> Result<LookupOutcome, AccessFailure> {
        Ok(LookupOutcome::None)
    }
    async fn resolve_target_identity(
        &self,
        _: &GitHubTarget,
    ) -> Result<TargetIdentity, AccessFailure> {
        if matches!(self.stage, FailureStage::Identity) {
            return Err(self.fail().await);
        }
        Ok(TargetIdentity {
            organization_id: Some(100),
            repository_id: None,
            repository_owner_id: None,
        })
    }
    fn allows_target(&self, _: &GitHubTarget) -> bool {
        true
    }
    async fn create_scale_set(
        &self,
        _: &ScaleSetIdentity,
        _: i64,
        _: &[Label],
    ) -> Result<EffectOutcome<ScaleSetView>, AccessFailure> {
        unreachable!("handoff must not create")
    }
    async fn establish_session(
        &self,
        _: i64,
        _: &str,
    ) -> Result<EffectOutcome<SessionHandle>, AccessFailure> {
        unreachable!("handoff must not establish a session")
    }
    async fn delete_session(&self, _: i64, _: &str) -> Result<(), AccessFailure> {
        unreachable!("handoff must not delete a session")
    }
    async fn poll_messages(
        &self,
        _: &SessionHandle,
        _: i64,
        _: i64,
    ) -> Result<PollOutcome, AccessFailure> {
        unreachable!("handoff must not poll")
    }
    async fn ack_message(&self, _: &SessionHandle, _: i64) -> Result<(), AccessFailure> {
        unreachable!("handoff must not acknowledge messages")
    }
    async fn acquire_jobs(
        &self,
        _: i64,
        _: &SessionHandle,
        _: &[i64],
    ) -> Result<EffectOutcome<Vec<i64>>, AccessFailure> {
        unreachable!("handoff must not acquire")
    }
    async fn generate_jit(
        &self,
        _: i64,
        _: &str,
    ) -> Result<EffectOutcome<JitConfig>, AccessFailure> {
        unreachable!("handoff must not mint JIT")
    }
    async fn get_runner_by_name(&self, _: i64, _: &str) -> Result<RunnerLookup, AccessFailure> {
        unreachable!("handoff must not lookup runners")
    }
    async fn remove_runner(&self, _: i64) -> Result<RemovalOutcome, AccessFailure> {
        unreachable!("handoff must not remove runners")
    }
    async fn list_runners(&self, _: i64) -> Result<Vec<RunnerRef>, AccessFailure> {
        unreachable!("handoff must not list runners")
    }
}
