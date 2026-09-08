#![allow(clippy::unwrap_used)] // test-only module
use std::sync::Arc;

use super::{run_handoff, HandoffProgress};
use shaula_core::ports::{AccessFailure, EffectOutcome, GitHubAccessPort};
use shaula_core::registry::ControlPlaneStore;

#[path = "handoff_memory.rs"]
pub mod memory_store;

pub use memory_store::MemoryStore;

// Fake GitHub port reporting a healthy authenticated read with no scale set.
#[derive(Default)]
pub struct HealthyGitHub {
    pub proof_calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl GitHubAccessPort for HealthyGitHub {
    async fn auth_context(&self) -> shaula_core::ports::AuthContext {
        shaula_core::ports::AuthContext {
            kind: shaula_core::auth::AuthKind::Pat,
        }
    }
    async fn resolve_runner_group(
        &self,
        _i: &shaula_core::github::ScaleSetIdentity,
    ) -> Result<i64, AccessFailure> {
        Ok(7)
    }
    async fn lookup_scale_set(
        &self,
        _i: &shaula_core::github::ScaleSetIdentity,
        _g: i64,
    ) -> Result<shaula_core::ports::LookupOutcome, AccessFailure> {
        Ok(shaula_core::ports::LookupOutcome::None)
    }
    async fn create_scale_set(
        &self,
        _: &shaula_core::github::ScaleSetIdentity,
        _: i64,
        _: &[shaula_core::github::Label],
    ) -> Result<EffectOutcome<shaula_core::ports::ScaleSetView>, AccessFailure> {
        unreachable!("handoff must never create")
    }
    async fn establish_session(
        &self,
        _: i64,
        _: &str,
    ) -> Result<EffectOutcome<shaula_core::ports::SessionHandle>, AccessFailure> {
        unreachable!("handoff must never establish a session")
    }
    async fn delete_session(&self, _: i64, _: &str) -> Result<(), AccessFailure> {
        Ok(())
    }
    async fn poll_messages(
        &self,
        _: &shaula_core::ports::SessionHandle,
        _: i64,
        _: i64,
    ) -> Result<shaula_core::ports::PollOutcome, AccessFailure> {
        unreachable!()
    }
    async fn ack_message(
        &self,
        _: &shaula_core::ports::SessionHandle,
        _: i64,
    ) -> Result<(), AccessFailure> {
        Ok(())
    }
    async fn acquire_jobs(
        &self,
        _: i64,
        _: &shaula_core::ports::SessionHandle,
        _: &[i64],
    ) -> Result<EffectOutcome<Vec<i64>>, AccessFailure> {
        unreachable!("handoff must never acquire")
    }
    async fn generate_jit(
        &self,
        _: i64,
        _: &str,
    ) -> Result<EffectOutcome<shaula_core::ports::JitConfig>, AccessFailure> {
        unreachable!("handoff must never mint JIT")
    }
    async fn get_runner_by_name(
        &self,
        _: i64,
        _: &str,
    ) -> Result<shaula_core::ports::RunnerLookup, AccessFailure> {
        Ok(shaula_core::ports::RunnerLookup::None)
    }
    async fn remove_runner(
        &self,
        _: i64,
    ) -> Result<shaula_core::ports::RemovalOutcome, AccessFailure> {
        Ok(shaula_core::ports::RemovalOutcome::AlreadyAbsent)
    }
    async fn list_runners(
        &self,
        _: i64,
    ) -> Result<Vec<shaula_core::ports::RunnerRef>, AccessFailure> {
        Ok(Vec::new())
    }
    fn allows_target(&self, _: &shaula_core::github::GitHubTarget) -> bool {
        true
    }
    async fn ensure_route_proof(&self) -> Result<shaula_core::ports::RouteProof, AccessFailure> {
        self.proof_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(shaula_core::ports::RouteProof {
            checked_at_unix_ms: 1_800_000_000_000,
            valid_until_unix_ms: 1_800_000_060_000,
            installation_id: 1,
            account_id: 1,
            organization_id: Some(1),
            repository_id: None,
            repository_owner_id: None,
        })
    }
    async fn resolve_target_identity(
        &self,
        _: &shaula_core::github::GitHubTarget,
    ) -> Result<shaula_core::ports::TargetIdentity, AccessFailure> {
        Ok(shaula_core::ports::TargetIdentity {
            organization_id: Some(1),
            repository_id: None,
            repository_owner_id: None,
        })
    }
}

#[tokio::test]
async fn handoff_advances_observed_tuple_without_remote_effects() {
    let store = Arc::new(MemoryStore::default());
    store.handoffs.lock().await.insert(
        "f1".into(),
        shaula_core::registry::AuthHandoffRow {
            fleet_key: "f1".into(),
            desired: ("prod-app".into(), 3),
            observed: Some(("prod-app".into(), 2)),
            state: "Pending".into(),
            cleanup_only: false,
            blocked_reason: None,
            retry_at: None,
        },
    );
    let identity = shaula_core::github::ScaleSetIdentity {
        target: shaula_core::github::GitHubTarget::organization("example-org").unwrap(),
        runner_group: "Default".into(),
        scale_set_name: "shaula-x64".into(),
    };
    let store_dyn: Arc<dyn ControlPlaneStore> = store.clone();
    let github: Arc<dyn GitHubAccessPort> = Arc::new(HealthyGitHub::default());
    let progress = run_handoff(
        &store_dyn,
        "f1",
        &github,
        &("prod-app".into(), 3),
        &identity,
        None,
        100,
    )
    .await
    .unwrap();
    assert_eq!(progress, HandoffProgress::Acknowledged);

    let row = store.handoff_get("f1").await.unwrap().unwrap();
    assert_eq!(row.observed, Some(("prod-app".into(), 3)));
    assert_eq!(row.state, "Observed");
}

#[tokio::test]
async fn handoff_blocks_without_fallback() {
    struct DeniedGitHub;
    #[async_trait::async_trait]
    impl GitHubAccessPort for DeniedGitHub {
        async fn auth_context(&self) -> shaula_core::ports::AuthContext {
            shaula_core::ports::AuthContext {
                kind: shaula_core::auth::AuthKind::Pat,
            }
        }
        async fn resolve_runner_group(
            &self,
            _identity: &shaula_core::github::ScaleSetIdentity,
        ) -> Result<i64, AccessFailure> {
            Err(AccessFailure::PermissionDenied)
        }
        async fn lookup_scale_set(
            &self,
            _identity: &shaula_core::github::ScaleSetIdentity,
            _runner_group_id: i64,
        ) -> Result<shaula_core::ports::LookupOutcome, AccessFailure> {
            unreachable!("group resolution failed first");
        }
        async fn create_scale_set(
            &self,
            _: &shaula_core::github::ScaleSetIdentity,
            _: i64,
            _: &[shaula_core::github::Label],
        ) -> Result<EffectOutcome<shaula_core::ports::ScaleSetView>, AccessFailure> {
            unreachable!();
        }
        async fn establish_session(
            &self,
            _: i64,
            _: &str,
        ) -> Result<EffectOutcome<shaula_core::ports::SessionHandle>, AccessFailure> {
            unreachable!();
        }
        async fn delete_session(&self, _: i64, _: &str) -> Result<(), AccessFailure> {
            Ok(())
        }
        async fn poll_messages(
            &self,
            _: &shaula_core::ports::SessionHandle,
            _: i64,
            _: i64,
        ) -> Result<shaula_core::ports::PollOutcome, AccessFailure> {
            unreachable!();
        }
        async fn ack_message(
            &self,
            _: &shaula_core::ports::SessionHandle,
            _: i64,
        ) -> Result<(), AccessFailure> {
            Ok(())
        }
        async fn acquire_jobs(
            &self,
            _: i64,
            _: &shaula_core::ports::SessionHandle,
            _: &[i64],
        ) -> Result<EffectOutcome<Vec<i64>>, AccessFailure> {
            unreachable!();
        }
        async fn generate_jit(
            &self,
            _: i64,
            _: &str,
        ) -> Result<EffectOutcome<shaula_core::ports::JitConfig>, AccessFailure> {
            unreachable!();
        }
        async fn get_runner_by_name(
            &self,
            _: i64,
            _: &str,
        ) -> Result<shaula_core::ports::RunnerLookup, AccessFailure> {
            Ok(shaula_core::ports::RunnerLookup::None)
        }
        async fn remove_runner(
            &self,
            _: i64,
        ) -> Result<shaula_core::ports::RemovalOutcome, AccessFailure> {
            Ok(shaula_core::ports::RemovalOutcome::AlreadyAbsent)
        }
        async fn list_runners(
            &self,
            _: i64,
        ) -> Result<Vec<shaula_core::ports::RunnerRef>, AccessFailure> {
            Ok(Vec::new())
        }
        fn allows_target(&self, _: &shaula_core::github::GitHubTarget) -> bool {
            false
        }
        async fn ensure_route_proof(
            &self,
        ) -> Result<shaula_core::ports::RouteProof, AccessFailure> {
            Err(AccessFailure::PermissionDenied)
        }
        async fn resolve_target_identity(
            &self,
            _: &shaula_core::github::GitHubTarget,
        ) -> Result<shaula_core::ports::TargetIdentity, AccessFailure> {
            unreachable!("group resolution failed first");
        }
    }

    let store = Arc::new(MemoryStore::default());
    store.handoffs.lock().await.insert(
        "f1".into(),
        shaula_core::registry::AuthHandoffRow {
            fleet_key: "f1".into(),
            desired: ("prod-app".into(), 3),
            observed: None,
            state: "Pending".into(),
            cleanup_only: false,
            blocked_reason: None,
            retry_at: None,
        },
    );
    let identity = shaula_core::github::ScaleSetIdentity {
        target: shaula_core::github::GitHubTarget::organization("example-org").unwrap(),
        runner_group: "Default".into(),
        scale_set_name: "shaula-x64".into(),
    };
    let store_dyn: Arc<dyn ControlPlaneStore> = store.clone();
    let github: Arc<dyn GitHubAccessPort> = Arc::new(DeniedGitHub);
    let progress = run_handoff(
        &store_dyn,
        "f1",
        &github,
        &("prod-app".into(), 3),
        &identity,
        None,
        500,
    )
    .await
    .unwrap();
    assert_eq!(progress, HandoffProgress::Blocked);
    let row = store.handoff_get("f1").await.unwrap().unwrap();
    assert_eq!(
        row.observed, None,
        "403 must not advance the observed tuple"
    );
    assert_eq!(row.state, "Blocked");
}
