use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::time::Duration;

use sea_orm::ConnectionTrait;
use shaula_core::error::{CoreError, ReasonCode};
use shaula_core::ports::Clock;
use shaula_daemon::service::ControlPlane;

use super::{registry::*, support::*};

struct PausedCommit {
    reached: mpsc::SyncSender<()>,
    release: Mutex<mpsc::Receiver<()>>,
    failed: AtomicBool,
}

impl Clock for PausedCommit {
    fn now_unix_ms(&self) -> i64 {
        let released = self.reached.send(()).is_ok()
            && self
                .release
                .lock()
                .is_ok_and(|r| r.recv_timeout(Duration::from_secs(5)).is_ok());
        self.failed.store(!released, Ordering::SeqCst);
        1_800_000_003_000
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replay_recheck_storage_failure_is_an_error_not_an_accepted_noop() -> TestResult {
    let fixture = Fixture::new().await?;
    let original = success(
        Kind::Pool
            .publish(
                &fixture.service,
                &actor(),
                &fixture.digest,
                0,
                Conditions::create(),
            )
            .await?,
    )?;
    let audits = fixture.count("audit_records").await?;
    let (reached, arrival) = mpsc::sync_channel(1);
    let (release, resume) = mpsc::sync_channel(1);
    let clock = Arc::new(PausedCommit {
        reached,
        release: Mutex::new(resume),
        failed: AtomicBool::new(false),
    });
    let service = ControlPlane::new(
        fixture.store.clone(),
        clock.clone(),
        b"test-bindings-key".to_vec(),
        100,
        fixture.engine.clone(),
    );
    let conditions = Conditions::replace(&original, "unavailable")?;
    let digest = fixture.digest.clone();
    let pending = tokio::spawn(async move {
        Kind::Pool
            .publish(&service, &actor(), &digest, 0, conditions)
            .await
    });
    arrival.recv_timeout(Duration::from_secs(5))?;
    // Both the commit's insert and the subsequent read now fail; the first
    // idempotency lookup and admission completed before this schema fault.
    fixture
        .db
        .execute_unprepared("ALTER TABLE idempotency_records RENAME TO unavailable_idempotency")
        .await?;
    release.send(())?;
    let outcome = pending.await?;
    fixture
        .db
        .execute_unprepared("ALTER TABLE unavailable_idempotency RENAME TO idempotency_records")
        .await?;
    let error = outcome
        .err()
        .ok_or("unavailable replay must not report success")?;
    assert_eq!(
        error
            .downcast_ref::<CoreError>()
            .ok_or("storage error")?
            .code,
        ReasonCode::StorageUnavailable
    );
    assert!(!clock.failed.load(Ordering::SeqCst));
    assert_eq!(fixture.count("audit_records").await?, audits);
    assert_eq!(Kind::Pool.revision(&fixture).await?, 1);
    Ok(())
}
