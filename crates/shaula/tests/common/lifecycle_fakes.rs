use shaula_core::{github::*, ports::*};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex,
};

#[derive(Default)]
pub struct GitHub {
    pub denied: AtomicBool,
    /// Route-proof gate control (spec 0011 §5.3): when set,
    /// ensure_route_proof fails, so every new management effect blocks.
    pub proof_denied: AtomicBool,
    pub busy: AtomicBool,
    pub removals: AtomicUsize,
    pub effects: AtomicUsize,
    pub jit_definite: AtomicBool,
    pub jit_uncertain: AtomicBool,
    pub runners: Mutex<Vec<RunnerRef>>,
}
#[async_trait::async_trait]
impl GitHubAccessPort for GitHub {
    async fn auth_context(&self) -> AuthContext {
        AuthContext {
            kind: shaula_core::auth::AuthKind::Pat,
        }
    }
    async fn resolve_runner_group(&self, _: &ScaleSetIdentity) -> Result<i64, AccessFailure> {
        if self.denied.load(Ordering::SeqCst) {
            Err(AccessFailure::PermissionDenied)
        } else {
            Ok(7)
        }
    }
    async fn lookup_scale_set(
        &self,
        i: &ScaleSetIdentity,
        _: i64,
    ) -> Result<LookupOutcome, AccessFailure> {
        Ok(LookupOutcome::ExactlyOne(ScaleSetView {
            id: 42,
            name: i.scale_set_name.clone(),
            runner_group_id: 7,
            runner_group_name: i.runner_group.clone(),
            labels: vec![Label {
                name: "shaula-x64".into(),
                label_type: "Customer".into(),
            }],
        }))
    }
    async fn list_runners(&self, _: i64) -> Result<Vec<RunnerRef>, AccessFailure> {
        Ok(self.runners.lock().unwrap().clone())
    }
    async fn remove_runner(&self, id: i64) -> Result<RemovalOutcome, AccessFailure> {
        self.removals.fetch_add(1, Ordering::SeqCst);
        Ok(if self.busy.load(Ordering::SeqCst) {
            RemovalOutcome::JobStillRunning
        } else {
            self.runners
                .lock()
                .unwrap()
                .retain(|runner| runner.id != id);
            RemovalOutcome::Removed
        })
    }
    async fn create_scale_set(
        &self,
        _: &ScaleSetIdentity,
        _: i64,
        _: &[Label],
    ) -> Result<EffectOutcome<ScaleSetView>, AccessFailure> {
        self.effects.fetch_add(1, Ordering::SeqCst);
        Err(AccessFailure::PermissionDenied)
    }
    async fn establish_session(
        &self,
        _: i64,
        _: &str,
    ) -> Result<EffectOutcome<SessionHandle>, AccessFailure> {
        unreachable!()
    }
    async fn delete_session(&self, _: i64, _: &str) -> Result<(), AccessFailure> {
        unreachable!()
    }
    async fn poll_messages(
        &self,
        _: &SessionHandle,
        _: i64,
        _: i64,
    ) -> Result<PollOutcome, AccessFailure> {
        unreachable!()
    }
    async fn ack_message(&self, _: &SessionHandle, _: i64) -> Result<(), AccessFailure> {
        unreachable!()
    }
    async fn acquire_jobs(
        &self,
        _: i64,
        _: &SessionHandle,
        _: &[i64],
    ) -> Result<EffectOutcome<Vec<i64>>, AccessFailure> {
        unreachable!()
    }
    async fn generate_jit(
        &self,
        scale_set_id: i64,
        name: &str,
    ) -> Result<EffectOutcome<JitConfig>, AccessFailure> {
        self.effects.fetch_add(1, Ordering::SeqCst);
        if self.jit_definite.load(Ordering::SeqCst) {
            let runner = RunnerRef {
                id: 77,
                name: name.into(),
                scale_set_id,
            };
            self.runners.lock().unwrap().push(runner.clone());
            return Ok(EffectOutcome::Definite(JitConfig {
                runner,
                encoded: "ephemeral-jit".into(),
            }));
        }
        if self.jit_uncertain.load(Ordering::SeqCst) {
            return Ok(EffectOutcome::Uncertain {
                summary: "response lost".into(),
            });
        }
        Err(AccessFailure::PermissionDenied)
    }
    async fn get_runner_by_name(&self, _: i64, _: &str) -> Result<RunnerLookup, AccessFailure> {
        unreachable!()
    }
    fn allows_target(&self, _: &GitHubTarget) -> bool {
        true
    }
    async fn resolve_target_identity(
        &self,
        _: &GitHubTarget,
    ) -> Result<TargetIdentity, AccessFailure> {
        Ok(TargetIdentity {
            organization_id: Some(1),
            repository_id: None,
            repository_owner_id: None,
        })
    }
    async fn ensure_route_proof(&self) -> Result<RouteProof, AccessFailure> {
        if self.proof_denied.load(Ordering::SeqCst) {
            Err(AccessFailure::PermissionDenied)
        } else {
            Ok(RouteProof {
                checked_at_unix_ms: 1_800_000_000_000,
                valid_until_unix_ms: 1_800_000_060_000,
                installation_id: 1,
                account_id: 1,
                organization_id: Some(1),
                repository_id: None,
                repository_owner_id: None,
            })
        }
    }
}
