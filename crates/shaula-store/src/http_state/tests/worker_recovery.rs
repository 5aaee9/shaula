use super::*;
use crate::http_state::{SqliteWorkerJournal, WorkerAdmissions};
use shaula_core::{
    state_backend::{StateBackend, StateError},
    worker::{journal::WorkerJournal, FenceOutcome, ProcessIdentity},
};
use std::sync::Arc;

#[tokio::test]
async fn worker_recovery_rotates_both_capabilities_and_audits_orphan_lock() -> TestResult {
    let f = Fixture::new().await?;
    let (admission, access, state) = super::worker::admit(&f).await?;
    f.backend.worker_launch_pending(&access).await?;
    f.backend
        .worker_register(
            &access,
            &ProcessIdentity {
                host_boot: "host:boot".into(),
                process_id: 99,
                started: "42".into(),
                containment: "/cgroup/attempt".into(),
            },
        )
        .await?;
    f.backend.lock(&state, lock("orphan")?).await?;
    let record = f
        .backend
        .recovery_records()
        .await?
        .pop()
        .ok_or("record absent")?;
    let admissions = Arc::new(WorkerAdmissions::new(4, 1)?);
    f.backend
        .recover_worker(&record, FenceOutcome::Fenced, &admissions, false)
        .await?;
    assert!(matches!(
        f.backend.read(&state).await,
        Err(StateError::Unauthorized)
    ));
    assert!(matches!(
        f.backend.authenticate_worker(&access).await,
        Err(StateError::Unauthorized)
    ));
    let fresh = admissions.take(admission.claim.generation_id)?;
    assert!(fresh.cleanup_only);
    assert_eq!(fresh.claim.worker_epoch, 2);
    let fresh_state = StateAccess {
        generation_id: fresh.claim.generation_id,
        capability: fresh.state,
    };
    f.backend.lock(&fresh_state, lock("new-owner")?).await?;
    assert!(f
        .backend
        .unlock(&state, lock("orphan")?.id())
        .await
        .is_err());
    let receipt = f
        .store
        .connection()
        .query_one(sql(
            "SELECT orphan_lock, outcome FROM lifecycle_fences WHERE worker_attempt = ?",
            vec![admission.claim.worker_attempt.to_string().into()],
        ))
        .await?
        .ok_or("fence receipt absent")?;
    assert_eq!(receipt.try_get::<String>("", "outcome")?, "fenced");
    assert_eq!(
        LockInfo::parse(&receipt.try_get::<Vec<u8>>("", "orphan_lock")?)?.id(),
        lock("orphan")?.id()
    );
    let journal = SqliteWorkerJournal::new(f.backend.clone(), admissions);
    assert!(journal
        .admission(admission.claim.generation_id)
        .await
        .is_err());
    Ok(())
}

#[tokio::test]
async fn unknown_worker_fence_never_issues_replacement_or_releases_occupancy() -> TestResult {
    let f = Fixture::new().await?;
    let (admission, access, state) = super::worker::admit(&f).await?;
    f.backend.worker_launch_pending(&access).await?;
    let record = f
        .backend
        .recovery_records()
        .await?
        .pop()
        .ok_or("record absent")?;
    let admissions = WorkerAdmissions::new(4, 1)?;
    f.backend
        .recover_worker(&record, FenceOutcome::Unknown, &admissions, false)
        .await?;
    assert!(admissions.take(admission.claim.generation_id).is_err());
    assert!(f.backend.read(&state).await.is_err());
    assert_eq!(
        f.store
            .generation_get(&admission.claim.generation_id.to_string())
            .await?
            .ok_or("generation absent")?
            .state,
        "Quarantined"
    );
    Ok(())
}

#[tokio::test]
async fn worker_rotation_failure_rolls_back_credentials_lock_and_receipt() -> TestResult {
    let f = Fixture::new().await?;
    let (admission, _, state) = super::worker::admit(&f).await?;
    f.backend.lock(&state, lock("old")?).await?;
    let record = f
        .backend
        .recovery_records()
        .await?
        .pop()
        .ok_or("record absent")?;
    f.store.connection().execute_unprepared("CREATE TRIGGER fail_rotation BEFORE UPDATE OF worker_epoch ON generation_http_state BEGIN SELECT RAISE(ABORT,'injected rotation failure'); END").await?;
    let admissions = WorkerAdmissions::new(4, 1)?;
    assert!(f
        .backend
        .recover_worker(&record, FenceOutcome::Fenced, &admissions, false)
        .await
        .is_err());
    assert!(admissions.take(admission.claim.generation_id).is_err());
    assert!(f.backend.read(&state).await?.is_some());
    assert!(matches!(
        f.backend.lock(&state, lock("new")?).await,
        Err(StateError::Locked(_))
    ));
    let receipt = f
        .store
        .connection()
        .query_one(sql(
            "SELECT worker_attempt FROM lifecycle_fences WHERE worker_attempt = ?",
            vec![admission.claim.worker_attempt.to_string().into()],
        ))
        .await?;
    assert!(receipt.is_none());
    Ok(())
}

#[tokio::test]
async fn quarantine_stays_closed_after_verified_fence_but_operator_finalize_is_enabled(
) -> TestResult {
    let f = Fixture::new().await?;
    let (admission, access, _) = super::worker::admit(&f).await?;
    f.backend.worker_launch_pending(&access).await?;
    let record = f
        .backend
        .recovery_records()
        .await?
        .pop()
        .ok_or("worker missing")?;
    let admissions = WorkerAdmissions::new(4, 1)?;
    f.backend
        .recover_worker(&record, FenceOutcome::Unknown, &admissions, false)
        .await?;
    let id = admission.claim.generation_id.to_string();
    let request = Uuid::new_v4().to_string();
    let tx = f.store.begin().await?;
    assert!(SqliteStateBackend::operator_seal_on(&tx, &id, &request, 10)
        .await
        .is_err());
    tx.rollback().await?;
    let record = f
        .backend
        .recovery_records()
        .await?
        .pop()
        .ok_or("quarantine missing")?;
    f.backend
        .recover_worker(&record, FenceOutcome::Fenced, &admissions, false)
        .await?;
    assert!(admissions.take(admission.claim.generation_id).is_err());
    assert_eq!(
        f.store
            .generation_get(&id)
            .await?
            .ok_or("generation missing")?
            .state,
        "Quarantined"
    );
    let tx = f.store.begin().await?;
    SqliteStateBackend::operator_seal_on(&tx, &id, &request, 11).await?;
    let row = tx
        .query_one(sql(
            "SELECT completion_receipt FROM lifecycle_workers WHERE generation_id = ?",
            vec![id.into()],
        ))
        .await?
        .ok_or("receipt missing")?;
    let receipt: shaula_core::worker::CompletionReceipt =
        serde_json::from_str(&row.try_get::<String>("", "completion_receipt")?)?;
    assert_eq!(
        receipt.kind,
        shaula_core::worker::CompletionKind::OperatorAttested
    );
    tx.rollback().await?;
    Ok(())
}
