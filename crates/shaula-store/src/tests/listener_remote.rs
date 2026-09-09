//! Controllable queue service for real SQLite listener tests.

use shaula_core::{github::*, ports::*};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use tokio::sync::{Mutex, Notify};

#[derive(Default)]
pub(super) struct Remote {
    pub created: AtomicUsize,
    pub deleted: Mutex<Vec<String>>,
    pub events: Mutex<Vec<String>>,
    pub candidate_delete_failures: Mutex<VecDeque<AccessFailure>>,
    pub expire_poll: AtomicBool,
    pub block_poll: AtomicBool,
    pub poll_started: Notify,
    pub poll_release: Notify,
}

#[derive(Default)]
pub(super) struct TestClock(pub AtomicI64);
impl Clock for TestClock {
    fn now_unix_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl GitHubAccessPort for Remote {
    async fn auth_context(&self) -> AuthContext {
        AuthContext {
            kind: shaula_core::auth::AuthKind::GithubApp,
        }
    }
    async fn resolve_runner_group(&self, _: &ScaleSetIdentity) -> Result<i64, AccessFailure> {
        Ok(7)
    }
    async fn lookup_scale_set(
        &self,
        _: &ScaleSetIdentity,
        _: i64,
    ) -> Result<LookupOutcome, AccessFailure> {
        Ok(LookupOutcome::None)
    }
    async fn create_scale_set(
        &self,
        _: &ScaleSetIdentity,
        _: i64,
        _: &[Label],
    ) -> Result<EffectOutcome<ScaleSetView>, AccessFailure> {
        Err(AccessFailure::PermissionDenied)
    }
    async fn establish_session(
        &self,
        _: i64,
        _: &str,
    ) -> Result<EffectOutcome<SessionHandle>, AccessFailure> {
        let number = self.created.fetch_add(1, Ordering::SeqCst) + 1;
        self.events
            .lock()
            .await
            .push(format!("create:remote-{number}"));
        Ok(EffectOutcome::Definite(SessionHandle {
            session_id: format!("remote-{number}"),
            message_queue_url: "https://queue.test/messages".into(),
            message_queue_access_token: "queue-secret".into(),
            initial_statistics: StatisticsSnapshot::default(),
        }))
    }
    async fn delete_session(&self, _: i64, session_id: &str) -> Result<(), AccessFailure> {
        self.deleted.lock().await.push(session_id.to_owned());
        self.events
            .lock()
            .await
            .push(format!("delete:{session_id}"));
        if session_id.starts_with("remote-") {
            if let Some(failure) = self.candidate_delete_failures.lock().await.pop_front() {
                return Err(failure);
            }
        }
        Ok(())
    }
    async fn poll_messages(
        &self,
        _: &SessionHandle,
        _: i64,
        _: i64,
    ) -> Result<PollOutcome, AccessFailure> {
        self.poll_started.notify_one();
        if self.block_poll.load(Ordering::SeqCst) {
            self.poll_release.notified().await;
        }
        Ok(if self.expire_poll.swap(false, Ordering::SeqCst) {
            PollOutcome::SessionExpired
        } else {
            PollOutcome::NoMessage
        })
    }
    async fn ack_message(&self, _: &SessionHandle, _: i64) -> Result<(), AccessFailure> {
        Ok(())
    }
    async fn acquire_jobs(
        &self,
        _: i64,
        _: &SessionHandle,
        ids: &[i64],
    ) -> Result<EffectOutcome<Vec<i64>>, AccessFailure> {
        Ok(EffectOutcome::Definite(ids.to_vec()))
    }
    async fn generate_jit(
        &self,
        _: i64,
        _: &str,
    ) -> Result<EffectOutcome<JitConfig>, AccessFailure> {
        Err(AccessFailure::PermissionDenied)
    }
    async fn get_runner_by_name(&self, _: i64, _: &str) -> Result<RunnerLookup, AccessFailure> {
        Ok(RunnerLookup::None)
    }
    async fn remove_runner(&self, _: i64) -> Result<RemovalOutcome, AccessFailure> {
        Ok(RemovalOutcome::AlreadyAbsent)
    }
    async fn list_runners(&self, _: i64) -> Result<Vec<RunnerRef>, AccessFailure> {
        Ok(Vec::new())
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
        Ok(RouteProof {
            checked_at_unix_ms: 0,
            valid_until_unix_ms: i64::MAX,
            installation_id: 11,
            account_id: 100,
            organization_id: Some(100),
            repository_id: None,
            repository_owner_id: None,
        })
    }
}
