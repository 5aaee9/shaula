use super::*;
use crate::http_state::WorkerAdmissions;

#[tokio::test]
async fn concurrent_admission_preserves_recovery_reserve_and_unknown_worker_occupancy() -> TestResult
{
    let f = Fixture::new().await?;
    f.store
        .connection()
        .execute(sql(
            "UPDATE runner_generations SET state = 'Destroyed' WHERE id = ?",
            vec![f.claim.generation_id.to_string().into()],
        ))
        .await?;
    f.backend.activate().await?;
    let admissions = WorkerAdmissions::new(2, 1)?;
    let admit = async |id: Uuid| {
        let tx = f.store.begin().await?;
        Store::generation_insert_on(&tx, record(id)).await?;
        let admission = admissions.admit_on(&tx, &id.to_string()).await;
        if admission.is_ok() {
            tx.commit().await?;
        } else {
            tx.rollback().await?;
        }
        TestResult::Ok(admission)
    };
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    let (a, b) = tokio::join!(admit(first), admit(second));
    let (a, b) = (a?, b?);
    assert_ne!(a.is_ok(), b.is_ok(), "one slot reserved for recovery");
    let (accepted, rejected) = if a.is_ok() {
        (first, second)
    } else {
        (second, first)
    };
    assert!(f
        .store
        .generation_get(&rejected.to_string())
        .await?
        .is_none());
    f.store
        .connection()
        .execute(sql(
            "UPDATE lifecycle_workers SET phase = 'quarantined' WHERE generation_id = ?",
            vec![accepted.to_string().into()],
        ))
        .await?;
    assert!(
        admit(Uuid::new_v4()).await?.is_err(),
        "unknown process still consumes capacity"
    );
    Ok(())
}
