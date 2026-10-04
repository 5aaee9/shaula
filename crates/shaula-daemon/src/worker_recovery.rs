use shaula_core::worker::recovery::RecoveryJournal;
use shaula_core::worker::{
    cleanup::{CleanupOutcome, WorkspaceReaper},
    Executor, FenceOutcome,
};
use std::path::Path;

/// Restart reaper for a crash after terminal commit but before local cleanup.
/// A durable receipt alone is not authority to race a still-live worker tree.
pub async fn reap(
    backend: &dyn RecoveryJournal,
    executor: &dyn Executor,
    reaper: &dyn WorkspaceReaper,
    artifact_root: &Path,
) -> Result<(), String> {
    let mut cursor = String::new();
    while let Some(record) = backend
        .next_completed_workspace(&cursor, artifact_root)
        .await
        .map_err(|_| "terminal cleanup scan unavailable")?
    {
        let receipt = record.cleanup.receipt.clone();
        cursor = receipt.claim.generation_id.to_string();
        if let Some(identity) = &record.process {
            if executor.stop_and_fence(identity).await != FenceOutcome::Fenced {
                continue;
            }
        }
        if matches!(
            reaper.reap(record.cleanup).await,
            Ok(CleanupOutcome::Reaped)
        ) {
            backend
                .workspace_reaped(&receipt)
                .await
                .map_err(|_| "terminal cleanup receipt unavailable")?;
            if let Some(identity) = &record.process {
                let _ = executor.release_fenced(identity).await;
            }
        }
    }
    Ok(())
}

/// Reconcile only after the daemon owns the host/database lifecycle lock.
pub async fn recover(journal: &dyn RecoveryJournal, executor: &dyn Executor) -> Result<(), String> {
    let records = journal
        .recovery_records()
        .await
        .map_err(|_| "worker recovery scan failed")?;
    for record in records {
        let outcome = match record.identity() {
            Ok(_) if record.phase == "fenced" => FenceOutcome::Fenced,
            Ok(Some(identity)) => executor.stop_and_fence(&identity).await,
            Ok(None) if matches!(record.phase.as_str(), "admitted" | "fenced") => {
                FenceOutcome::Fenced
            }
            // Pre-exec crash: classified by the exact containment, never
            // by an empty in-memory child list.
            Ok(None) if record.phase == "launch_pending" => match record.claim() {
                Ok(claim) => executor.fence_unregistered(&claim).await,
                Err(_) => FenceOutcome::Unknown,
            },
            _ => FenceOutcome::Unknown,
        };
        let workspace = std::path::Path::new(&record.workspace_path);
        let emergency = [
            "errored.tfstate",
            "terraform.tfstate",
            "terraform.tfstate.backup",
        ]
        .iter()
        .any(|name| workspace.join(name).exists());
        journal
            .recover_worker(&record, outcome, emergency)
            .await
            .map_err(|_| "worker recovery classification failed")?;
    }
    Ok(())
}
