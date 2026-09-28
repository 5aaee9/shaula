//! LW-11: crash in the pre-exec window (launch_pending, no durable identity).
use super::*;
use crate::http_state::WorkerAdmissions;
use shaula_core::{state_backend::StateBackend, worker::FenceOutcome};

#[tokio::test]
async fn fenced_pre_exec_crash_rotates_to_cleanup_only_without_create() -> TestResult {
    let f = Fixture::new().await?;
    let (admission, access, state) = super::worker::admit(&f).await?;
    f.backend.worker_launch_pending(&access).await?;
    let record = f
        .backend
        .recovery_records()
        .await?
        .pop()
        .ok_or("record absent")?;
    assert_eq!(record.phase, "launch_pending");
    assert!(record.identity()?.is_none());
    assert!(!record.create_started);
    let admissions = WorkerAdmissions::new(4, 1)?;
    f.backend
        .recover_worker(&record, FenceOutcome::Fenced, &admissions, false)
        .await?;
    // The pre-exec capabilities never left the daemon, yet are still rotated.
    assert!(f.backend.read(&state).await.is_err());
    let fresh = admissions.take(admission.claim.generation_id)?;
    assert!(fresh.cleanup_only);
    assert_eq!(fresh.claim.worker_epoch, admission.claim.worker_epoch + 1);
    assert_ne!(fresh.claim.worker_attempt, admission.claim.worker_attempt);
    let generation = f
        .store
        .generation_get(&admission.claim.generation_id.to_string())
        .await?
        .ok_or("generation absent")?;
    assert_ne!(generation.state, "Quarantined");
    assert_ne!(generation.state, "Destroyed");
    Ok(())
}

#[tokio::test]
async fn unknown_pre_exec_crash_quarantines_and_retains_occupancy() -> TestResult {
    let f = Fixture::new().await?;
    let (admission, access, _) = super::worker::admit(&f).await?;
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
    let generation = f
        .store
        .generation_get(&admission.claim.generation_id.to_string())
        .await?
        .ok_or("generation absent")?;
    assert_eq!(generation.state, "Quarantined");
    Ok(())
}
