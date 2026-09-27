use super::*;
use crate::http_state::{SqliteWorkerJournal, WorkerAdmissions};
use shaula_core::state_backend::StateBackend;
use shaula_core::worker::{journal::WorkerJournal, FenceOutcome, ProcessIdentity};
use std::sync::Arc;

#[tokio::test]
async fn fenced_partial_create_cannot_self_certify_state_completeness() -> TestResult {
    for proof in ["missing", "foreign", "rolled-back", "verified"] {
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
        assert!(journal.protected_input(&access).await?.is_none());
        journal.retain_input(&access, b"{}".to_vec()).await?;
        assert_eq!(
            journal.protected_input(&access).await?,
            Some(b"{}".to_vec())
        );
        f.backend.note_create_starting(&admission.claim).await?;
        let lineage = f
            .backend
            .read(&state_access)
            .await?
            .ok_or("state missing")?
            .document
            .lineage()
            .to_owned();
        // A was recorded, but a remote create of B could still be absent from
        // state. Deleting A must not make that partial inventory complete.
        f.backend.lock(&state_access, lock("apply")?).await?;
        for (serial, live) in [(1, true), (2, false)] {
            f.backend
                .write(
                    &state_access,
                    lock("apply")?.id(),
                    state(serial, &lineage, live)?,
                )
                .await?;
        }
        f.backend.unlock(&state_access, lock("apply")?.id()).await?;
        if proof != "missing" {
            let result = serde_json::json!({
                "state_lineage": if proof == "foreign" { "foreign" } else { &lineage },
                "state_serial": if proof == "rolled-back" { 3 } else { 1 },
            })
            .to_string();
            f.store
                .generation_set_result(&id, &result, "fixture", 3)
                .await?;
        }
        assert_eq!(
            journal.verify_cleanup(&access).await.is_ok(),
            proof == "verified"
        );
        // Local fencing/command-ended can clear command fields. It cannot
        // manufacture the missing durable successful Create result.
        f.store.connection().execute(sql(
            "UPDATE lifecycle_workers SET phase = 'fenced', active_command = NULL, handover = NULL WHERE generation_id = ?",
            vec![id.clone().into()],
        )).await?;
        let record = f
            .backend
            .recovery_records()
            .await?
            .pop()
            .ok_or("record missing")?;
        f.backend
            .recover_worker(&record, FenceOutcome::Fenced, &admissions, false)
            .await?;
        assert_eq!(
            admissions.take(admission.claim.generation_id).is_ok(),
            proof == "verified"
        );
        if proof != "verified" {
            assert_eq!(
                f.store
                    .generation_get(&id)
                    .await?
                    .ok_or("generation missing")?
                    .state,
                "Quarantined"
            );
            assert!(f.backend.read(&state_access).await.is_err());
            assert!(f.backend.complete_generation(&id, 4).await.is_err());
        }
    }
    Ok(())
}
