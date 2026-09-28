use shaula_core::{
    state_backend::{StateBackend, StateError},
    worker::{CompletionKind, ControlAccess, ProcessIdentity, WorkerAdmission},
};

use super::*;

pub(super) async fn admit(
    f: &Fixture,
) -> TestResult<(WorkerAdmission, ControlAccess, StateAccess)> {
    let admission = WorkerAdmission::new(Uuid::new_v4());
    let tx = f.store.begin().await?;
    Store::generation_insert_on(&tx, record(admission.claim.generation_id)).await?;
    SqliteStateBackend::admit_worker_on(&tx, &admission).await?;
    tx.commit().await?;
    let access = ControlAccess {
        claim: admission.claim.clone(),
        capability: admission.control.clone(),
    };
    let state = StateAccess {
        generation_id: admission.claim.generation_id,
        capability: admission.state.clone(),
    };
    Ok((admission, access, state))
}

#[tokio::test]
async fn admission_rollback_cannot_leave_partial_state_or_claim() -> TestResult {
    let f = Fixture::new().await?;
    let admission = WorkerAdmission::new(Uuid::new_v4());
    let tx = f.store.begin().await?;
    Store::generation_insert_on(&tx, record(admission.claim.generation_id)).await?;
    SqliteStateBackend::admit_worker_on(&tx, &admission).await?;
    tx.rollback().await?;
    assert!(f
        .store
        .generation_get(&admission.claim.generation_id.to_string())
        .await?
        .is_none());
    let access = StateAccess {
        generation_id: admission.claim.generation_id,
        capability: admission.state,
    };
    assert!(matches!(
        f.backend.read(&access).await,
        Err(StateError::Unauthorized)
    ));
    Ok(())
}

#[tokio::test]
async fn initial_state_precedes_init_and_duplicate_launch_is_refused() -> TestResult {
    let f = Fixture::new().await?;
    let (admission, access, state) = admit(&f).await?;
    let snapshot = f.backend.read(&state).await?.ok_or("snapshot missing")?;
    assert_eq!(snapshot.revision, 1);
    assert_eq!(snapshot.document.serial(), 0);
    assert!(snapshot.document.managed_empty());
    assert!(Uuid::parse_str(snapshot.document.lineage()).is_ok());
    let (a, b) = tokio::join!(
        f.backend.worker_launch_pending(&access),
        f.backend.worker_launch_pending(&access)
    );
    assert_ne!(a.is_ok(), b.is_ok());
    let identity = ProcessIdentity {
        host_boot: "boot".into(),
        process_id: 123,
        started: "start".into(),
        containment: "guardian".into(),
        root_id: None,
    };
    f.backend.worker_register(&access, &identity).await?;
    f.backend.worker_register(&access, &identity).await?;
    let mut different = identity.clone();
    different.started = "reused-pid".into();
    assert!(f
        .backend
        .worker_register(&access, &different)
        .await
        .is_err());
    let mut stale = access;
    stale.claim.worker_epoch += 1;
    assert!(matches!(
        f.backend.authenticate_worker(&stale).await,
        Err(StateError::Unauthorized)
    ));
    f.backend.note_create_starting(&admission.claim).await?;
    Ok(())
}

#[tokio::test]
async fn never_started_completion_seals_releases_and_replays_together() -> TestResult {
    let f = Fixture::new().await?;
    let (admission, access, state) = admit(&f).await?;
    let id = admission.claim.generation_id.to_string();
    f.store
        .connection()
        .execute(sql(
            "UPDATE runner_generations SET state = 'Destroying' WHERE id = ?",
            vec![id.clone().into()],
        ))
        .await?;
    let request = Uuid::new_v4();
    let receipt = f
        .backend
        .complete_worker(&access, request, 1, CompletionKind::NeverStarted, 10)
        .await?;
    let replay = f
        .backend
        .complete_worker(&access, request, 1, CompletionKind::NeverStarted, 20)
        .await?;
    assert_eq!(receipt, replay);
    assert!(f
        .backend
        .complete_worker(&access, Uuid::new_v4(), 1, CompletionKind::NeverStarted, 20)
        .await
        .is_err());
    assert_eq!(
        f.store
            .generation_get(&id)
            .await?
            .ok_or("generation missing")?
            .state,
        "Destroyed"
    );
    assert!(matches!(
        f.backend.lock(&state, lock("late")?).await,
        Err(StateError::Sealed)
    ));
    Ok(())
}

#[tokio::test]
async fn failed_completion_keeps_state_writable_and_occupancy_held() -> TestResult {
    let f = Fixture::new().await?;
    let (admission, access, state) = admit(&f).await?;
    let id = admission.claim.generation_id.to_string();
    f.store
        .connection()
        .execute(sql(
            "UPDATE runner_generations SET state = 'Destroying' WHERE id = ?",
            vec![id.clone().into()],
        ))
        .await?;
    f.backend.lock(&state, lock("active")?).await?;
    assert!(f
        .backend
        .complete_worker(&access, Uuid::new_v4(), 1, CompletionKind::NeverStarted, 10)
        .await
        .is_err());
    assert_eq!(
        f.store
            .generation_get(&id)
            .await?
            .ok_or("generation missing")?
            .state,
        "Destroying"
    );
    f.backend.unlock(&state, lock("active")?.id()).await?;
    // Fail after the seal and receipt UPDATEs, at the last occupancy write.
    f.store.connection().execute_unprepared("CREATE TRIGGER fail_worker_terminal BEFORE UPDATE OF state ON runner_generations WHEN NEW.state = 'Destroyed' BEGIN SELECT RAISE(ABORT, 'injected completion failure'); END").await?;
    assert!(f
        .backend
        .complete_worker(&access, Uuid::new_v4(), 1, CompletionKind::NeverStarted, 10)
        .await
        .is_err());
    f.backend.lock(&state, lock("still-writable")?).await?;
    f.backend
        .unlock(&state, lock("still-writable")?.id())
        .await?;
    f.store
        .connection()
        .execute_unprepared("DROP TRIGGER fail_worker_terminal")
        .await?;
    f.backend
        .complete_worker(&access, Uuid::new_v4(), 1, CompletionKind::NeverStarted, 10)
        .await?;
    Ok(())
}
