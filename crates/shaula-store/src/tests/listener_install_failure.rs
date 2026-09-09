//! A successful remote session remains cleanup-owned until installation commits.

use super::{Harness, TestResult};
use sea_orm::ConnectionTrait;
use shaula_core::{ports::AccessFailure, registry::LifecycleStore};
use std::sync::atomic::Ordering;

async fn reject_installs(h: &Harness) -> TestResult {
    h.store
        .store()
        .connection()
        .execute_unprepared(
            "CREATE TRIGGER fail_session_install BEFORE INSERT ON fleet_sessions \
         BEGIN SELECT RAISE(ABORT,'injected session install failure'); END",
        )
        .await?;
    Ok(())
}

async fn allow_installs(h: &Harness) -> TestResult {
    h.store
        .store()
        .connection()
        .execute_unprepared("DROP TRIGGER fail_session_install")
        .await?;
    Ok(())
}

fn temporary_failure() -> AccessFailure {
    AccessFailure::Unavailable {
        summary: "injected delete failure".into(),
    }
}

#[tokio::test]
async fn failed_install_closes_the_successful_remote_session() -> TestResult {
    let h = Harness::new().await?;
    let listener = h.listener().await?;
    reject_installs(&h).await?;

    assert!(listener.ensure_session(42).await.is_err());
    assert!(h.store.session_get("fleet").await?.is_none());
    assert_eq!(h.remote.created.load(Ordering::SeqCst), 1);
    assert_eq!(
        *h.remote.events.lock().await,
        ["delete:original", "create:remote-1", "delete:remote-1"]
    );

    allow_installs(&h).await?;
    assert!(listener.ensure_session(42).await?.is_some());
    let session = h.store.session_get("fleet").await?.ok_or("session")?;
    assert_eq!(session.handle.session_id, "remote-2");
    Ok(())
}

#[tokio::test]
async fn failed_cleanup_is_retried_before_any_new_session_is_created() -> TestResult {
    for already_absent in [false, true] {
        let h = Harness::new().await?;
        let listener = h.listener().await?;
        let mut failures = h.remote.candidate_delete_failures.lock().await;
        failures.push_back(temporary_failure());
        if already_absent {
            failures.push_back(AccessFailure::SessionExpired);
        }
        drop(failures);
        reject_installs(&h).await?;

        assert!(listener.ensure_session(42).await.is_err());
        assert!(h.store.session_get("fleet").await?.is_none());
        allow_installs(&h).await?;
        // A normal reconcile during backoff must not POST or retry DELETE.
        assert!(listener.ensure_session(42).await?.is_none());
        assert_eq!(h.remote.created.load(Ordering::SeqCst), 1);
        assert_eq!(h.remote.events.lock().await.len(), 3);

        h.clock.0.store(1_000_000, Ordering::SeqCst);
        assert!(listener.ensure_session(42).await?.is_some());
        assert_eq!(
            *h.remote.events.lock().await,
            [
                "delete:original",
                "create:remote-1",
                "delete:remote-1",
                "delete:remote-1",
                "create:remote-2",
            ]
        );
        let session = h.store.session_get("fleet").await?.ok_or("session")?;
        assert_eq!(session.handle.session_id, "remote-2");
    }
    Ok(())
}

#[tokio::test]
async fn stopping_a_listener_retries_cleanup_of_its_uncommitted_session() -> TestResult {
    let h = Harness::new().await?;
    let listener = h.listener().await?;
    h.remote
        .candidate_delete_failures
        .lock()
        .await
        .push_back(temporary_failure());
    reject_installs(&h).await?;

    assert!(listener.ensure_session(42).await.is_err());
    assert!(listener.stop().await?);
    assert!(listener.stop().await?);
    assert!(h.store.session_get("fleet").await?.is_none());
    assert_eq!(h.remote.created.load(Ordering::SeqCst), 1);
    assert_eq!(
        *h.remote.events.lock().await,
        [
            "delete:original",
            "create:remote-1",
            "delete:remote-1",
            "delete:remote-1",
        ]
    );
    Ok(())
}
