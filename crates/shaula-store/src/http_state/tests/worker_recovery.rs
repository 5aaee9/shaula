use super::*;
use crate::http_state::{SqliteWorkerJournal, WorkerAdmissions};
use shaula_core::{
    state_backend::{StateBackend, StateError},
    worker::{journal::WorkerJournal, FenceOutcome, ProcessIdentity},
};
use std::sync::Arc;

#[tokio::test]
async fn completed_cleanup_survives_restart_before_ci_registration_removal() -> TestResult {
    for after_cleanup in ["unchanged", "late-write", "active-command", "lock"] {
        let f = Fixture::new().await?;
        let (admission, access, state_access) = super::worker::admit(&f).await?;
        let id = admission.claim.generation_id.to_string();
        f.backend.worker_launch_pending(&access).await?;
        f.backend
            .worker_register(
                &access,
                &ProcessIdentity {
                    host_boot: "host:boot".into(),
                    process_id: 99,
                    started: "42".into(),
                    containment: "/cgroup/attempt".into(),
                    root_id: None,
                },
            )
            .await?;
        let admissions = Arc::new(WorkerAdmissions::new(4, 1)?);
        let journal = SqliteWorkerJournal::new(f.backend.clone(), admissions.clone());
        journal.retain_input(&access, b"{}".to_vec()).await?;
        f.backend.note_create_starting(&admission.claim).await?;
        let lineage = f
            .backend
            .read(&state_access)
            .await?
            .ok_or("state missing")?
            .document
            .lineage()
            .to_owned();
        f.backend.lock(&state_access, lock("apply")?).await?;
        f.backend
            .write(
                &state_access,
                lock("apply")?.id(),
                state(1, &lineage, true)?,
            )
            .await?;
        f.backend
            .write(
                &state_access,
                lock("apply")?.id(),
                state(2, &lineage, false)?,
            )
            .await?;
        f.backend.unlock(&state_access, lock("apply")?.id()).await?;
        f.store
            .generation_set_result(
                &id,
                &serde_json::json!({
                    "state_lineage": lineage, "state_serial": 1,
                })
                .to_string(),
                "fixture",
                3,
            )
            .await?;
        journal.verify_cleanup(&access).await?;
        f.store.connection().execute(sql(
            "UPDATE runner_generations SET state = 'Destroying', resources_destroyed_at = 10 WHERE id = ?",
            vec![id.clone().into()],
        )).await?;
        if after_cleanup == "late-write" {
            f.backend.lock(&state_access, lock("late-write")?).await?;
            f.backend
                .write(
                    &state_access,
                    lock("late-write")?.id(),
                    state(3, &lineage, false)?,
                )
                .await?;
            f.backend
                .unlock(&state_access, lock("late-write")?.id())
                .await?;
        }
        if after_cleanup == "active-command" {
            f.store.connection().execute(sql(
                "UPDATE lifecycle_workers SET active_command = 'unresolved' WHERE generation_id = ?",
                vec![id.clone().into()],
            )).await?;
        }
        if after_cleanup == "lock" {
            f.backend.lock(&state_access, lock("late-lock")?).await?;
        }
        let record = f
            .backend
            .recovery_records()
            .await?
            .into_iter()
            .find(|r| r.generation_id == id)
            .ok_or("record absent")?;
        f.backend
            .recover_worker(&record, FenceOutcome::Fenced, &admissions, false)
            .await?;
        let fresh = admissions.take(admission.claim.generation_id)?;
        assert_eq!(fresh.claim.worker_epoch, 2);
        assert!(f.backend.read(&state_access).await.is_err());
        // The daemon has now removed the CI registration. No second Terraform
        // Destroy is needed, but only the exact verified empty revision counts.
        let completed = f.backend.complete_generation(&id, 20).await;
        if after_cleanup != "unchanged" {
            assert!(completed.is_err());
            assert_eq!(
                f.store
                    .generation_get(&id)
                    .await?
                    .ok_or("generation missing")?
                    .state,
                "Destroying"
            );
        } else {
            assert!(completed?);
            assert_eq!(
                f.store
                    .generation_get(&id)
                    .await?
                    .ok_or("generation missing")?
                    .state,
                "Destroyed"
            );
            let fresh_state = StateAccess {
                generation_id: fresh.claim.generation_id,
                capability: fresh.state,
            };
            assert!(matches!(
                f.backend.lock(&fresh_state, lock("late")?).await,
                Err(StateError::Sealed)
            ));
        }
    }
    Ok(())
}

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
                root_id: None,
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
