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
    /// With jit_uncertain, also create the runner entity first — modeling
    /// "the mint landed but the response was lost" (spec 0025 incident).
    pub jit_uncertain_lands: AtomicBool,
    /// Make exact-name lookups fail (classification-unavailable branch).
    pub runner_lookup_denied: AtomicBool,
    pub runners: Mutex<Vec<RunnerRef>>,
    pub labels: Mutex<Option<Vec<Label>>>,
    pub lookup_override: Mutex<Option<Result<LookupOutcome, AccessFailure>>>,
    pub create_override: Mutex<Option<Result<EffectOutcome<ScaleSetView>, AccessFailure>>>,
}
#[async_trait::async_trait]
impl GitHubAccessPort for GitHub {
    async fn auth_context(&self) -> AuthContext {
        AuthContext {
            kind: shaula_core::auth::AuthKind::GithubApp,
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
        if let Some(outcome) = self.lookup_override.lock().unwrap().clone() {
            return outcome;
        }
        Ok(LookupOutcome::ExactlyOne(ScaleSetView {
            id: 42,
            name: i.scale_set_name.clone(),
            runner_group_id: 7,
            runner_group_name: i.runner_group.clone(),
            labels: self.labels.lock().unwrap().clone().unwrap_or_else(|| {
                vec![Label {
                    name: "shaula-x64".into(),
                    label_type: "System".into(),
                }]
            }),
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
        self.create_override
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(Err(AccessFailure::PermissionDenied))
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
                status: "offline".to_string(),
            };
            self.runners.lock().unwrap().push(runner.clone());
            return Ok(EffectOutcome::Definite(JitConfig {
                runner,
                encoded: "ephemeral-jit".into(),
            }));
        }
        if self.jit_uncertain.load(Ordering::SeqCst) {
            if self.jit_uncertain_lands.load(Ordering::SeqCst) {
                self.runners.lock().unwrap().push(RunnerRef {
                    id: 78,
                    name: name.into(),
                    scale_set_id,
                    status: "offline".to_string(),
                });
            }
            return Ok(EffectOutcome::Uncertain {
                summary: "response lost".into(),
            });
        }
        Err(AccessFailure::PermissionDenied)
    }
    async fn get_runner_by_name(
        &self,
        _scale_set_id: i64,
        name: &str,
    ) -> Result<RunnerLookup, AccessFailure> {
        if self.runner_lookup_denied.load(Ordering::SeqCst) {
            return Err(AccessFailure::PermissionDenied);
        }
        let matches: Vec<_> = self
            .runners
            .lock()
            .unwrap()
            .iter()
            .filter(|runner| runner.name == name)
            .cloned()
            .collect();
        match matches.len() {
            0 => Ok(RunnerLookup::None),
            1 => Ok(RunnerLookup::ExactlyOne(matches[0].clone())),
            _ => Ok(RunnerLookup::Multiple),
        }
    }
    fn allows_target(&self, _: &GitHubTarget) -> bool {
        true
    }
    async fn resolve_target_identity(
        &self,
        _: &GitHubTarget,
    ) -> Result<TargetIdentity, AccessFailure> {
        Ok(TargetIdentity {
            organization_id: Some(100),
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
                installation_id: 11,
                account_id: 100,
                organization_id: Some(100),
                repository_id: None,
                repository_owner_id: None,
            })
        }
    }
}
