use shaula_core::{github::*, ports::*};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex,
};

#[derive(Default)]
pub struct GitHub {
    pub denied: AtomicBool,
    pub busy: AtomicBool,
    pub removals: AtomicUsize,
    pub effects: AtomicUsize,
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
    async fn remove_runner(&self, _: i64) -> Result<RemovalOutcome, AccessFailure> {
        self.removals.fetch_add(1, Ordering::SeqCst);
        Ok(if self.busy.load(Ordering::SeqCst) {
            RemovalOutcome::JobStillRunning
        } else {
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
        _: i64,
        _: &str,
    ) -> Result<EffectOutcome<JitConfig>, AccessFailure> {
        self.effects.fetch_add(1, Ordering::SeqCst);
        Err(AccessFailure::PermissionDenied)
    }
    async fn get_runner_by_name(&self, _: i64, _: &str) -> Result<RunnerLookup, AccessFailure> {
        unreachable!()
    }
    fn allows_target(&self, _: &GitHubTarget) -> bool {
        true
    }
}

pub struct Runtime;
#[async_trait::async_trait]
impl TemplateRuntimePort for Runtime {
    async fn create(
        &self,
        _: TemplateCreateRequest,
    ) -> Result<TemplateCreateResult, TemplateOutcomeError> {
        panic!("Create must be gated")
    }
    async fn destroy(
        &self,
        _: TemplateDestroyRequest,
    ) -> Result<DestroyClassification, TemplateOutcomeError> {
        panic!("Destroy must be gated")
    }
}
