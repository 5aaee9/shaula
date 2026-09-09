//! Production listener session orchestration with SQLite and controlled GitHub.

#[path = "listener_install_failure.rs"]
mod install_failure;
#[path = "listener_remote.rs"]
mod remote;

use sea_orm::ConnectionTrait;
use shaula_core::auth_context::ResolvedAuthContext;
use shaula_core::registry::{FleetRuntimeGuard, LifecycleStore};
use shaula_daemon::effect_gate::FleetEffectGates;
use shaula_daemon::listener::{FleetListener, ListenerConfig, ListenerDeps};
use std::sync::{atomic::Ordering, Arc};
use std::time::Duration;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
struct Harness {
    store: Arc<crate::registry_impl::SqliteControlPlane>,
    context: ResolvedAuthContext,
    remote: Arc<remote::Remote>,
    clock: Arc<remote::TestClock>,
    gates: Arc<FleetEffectGates>,
}

impl Harness {
    async fn new() -> TestResult<Self> {
        let (store, context) = super::runtime_sessions::established().await?;
        Ok(Self {
            store: Arc::new(crate::registry_impl::SqliteControlPlane::new(
                store,
                "unused".into(),
            )),
            context,
            remote: Arc::default(),
            clock: Arc::default(),
            gates: Arc::default(),
        })
    }
    async fn listener(&self) -> TestResult<Arc<FleetListener>> {
        let guard = super::session_support::request(
            self.store.store(),
            "fleet",
            "capture",
            42,
            &self.context,
        )
        .await?
        .guard;
        Ok(self.listener_with_guard(guard))
    }
    fn listener_with_guard(&self, guard: FleetRuntimeGuard) -> Arc<FleetListener> {
        Arc::new(FleetListener::new(
            ListenerConfig {
                fleet_key: "fleet".into(),
                guard,
                auth_context: self.context.clone(),
                max_capacity: 10,
            },
            ListenerDeps {
                store: self.store.clone(),
                github: self.remote.clone(),
                retained_clients: Default::default(),
                gates: self.gates.clone(),
                clock: self.clock.clone(),
            },
        ))
    }
}

#[tokio::test]
async fn expired_session_reconnects_with_a_new_epoch_after_backoff() -> TestResult {
    let h = Harness::new().await?;
    let listener = h.listener().await?;
    assert!(listener.ensure_session(42).await?.is_some());
    let first = h.store.session_get("fleet").await?.ok_or("session")?;
    h.remote.expire_poll.store(true, Ordering::SeqCst);
    listener.poll_once().await?;
    assert!(listener.ensure_session(42).await?.is_none());
    h.clock.0.store(1_000_000, Ordering::SeqCst);
    assert!(listener.ensure_session(42).await?.is_some());
    let second = h.store.session_get("fleet").await?.ok_or("session")?;
    assert!(second.epoch > first.epoch);
    assert_ne!(second.handle.session_id, first.handle.session_id);
    assert_eq!(h.remote.created.load(Ordering::SeqCst), 2);
    Ok(())
}

#[tokio::test]
async fn stale_listener_stop_cannot_close_a_replacement_with_the_same_fleet_head() -> TestResult {
    let h = Harness::new().await?;
    let stale = h.listener().await?;
    assert!(stale.ensure_session(42).await?.is_some());
    let old_epoch = h
        .store
        .session_get("fleet")
        .await?
        .ok_or("old session")?
        .epoch;
    let newer = h.listener().await?;
    assert!(newer.ensure_session(42).await?.is_some());
    let replacement = h.store.session_get("fleet").await?.ok_or("session")?;
    assert!(stale.stop().await?);
    assert!(stale.stop().await?);
    let retained = h
        .store
        .session_get("fleet")
        .await?
        .ok_or("replacement retained")?;
    assert_eq!(retained.epoch, replacement.epoch);
    assert_eq!(retained.handle.session_id, replacement.handle.session_id);
    assert!(!h
        .remote
        .deleted
        .lock()
        .await
        .contains(&replacement.handle.session_id));
    stale
        .observe_failure(old_epoch, shaula_core::error::ReasonCode::PermissionDenied)
        .await?;
    assert!(h
        .store
        .store()
        .fleet_get("fleet")
        .await?
        .ok_or("fleet")?
        .last_condition_reason
        .is_none());
    Ok(())
}

#[tokio::test]
async fn restart_during_decommission_closes_persisted_session_but_a_stale_head_does_not(
) -> TestResult {
    let h = Harness::new().await?;
    let stale = h.listener().await?;
    h.store.store().connection().execute_unprepared(
        "UPDATE fleets SET desired_revision=2,mutation_fence=2,deletion_marker=1 WHERE key='fleet'"
    ).await?;
    assert!(stale.stop().await?);
    assert!(h.store.session_get("fleet").await?.is_some());
    assert!(h.remote.deleted.lock().await.is_empty());
    let current = h.listener().await?;
    assert!(current.stop().await?);
    assert!(h.store.session_get("fleet").await?.is_none());
    assert_eq!(*h.remote.deleted.lock().await, vec!["original"]);
    Ok(())
}

#[tokio::test]
async fn a_long_poll_does_not_hold_the_effect_gate_or_block_session_replacement() -> TestResult {
    let h = Harness::new().await?;
    let listener = h.listener().await?;
    assert!(listener.ensure_session(42).await?.is_some());
    h.remote.block_poll.store(true, Ordering::SeqCst);
    let polling = tokio::spawn({
        let listener = listener.clone();
        async move { listener.poll_once().await }
    });
    tokio::time::timeout(Duration::from_secs(2), h.remote.poll_started.notified()).await?;
    let gate =
        tokio::time::timeout(Duration::from_secs(2), h.gates.acquire_exclusive("fleet")).await?;
    drop(gate);
    let replacement = h.listener().await?;
    assert!(
        tokio::time::timeout(Duration::from_secs(2), replacement.ensure_session(42))
            .await??
            .is_some()
    );
    let latest = h
        .store
        .session_get("fleet")
        .await?
        .ok_or("latest session")?;
    h.remote.poll_release.notify_one();
    tokio::time::timeout(Duration::from_secs(2), polling).await???;
    assert_eq!(
        h.store.session_get("fleet").await?.ok_or("session")?.epoch,
        latest.epoch
    );
    Ok(())
}
