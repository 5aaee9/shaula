use shaula_core::worker::{
    cleanup::{CleanupOutcome, WorkspaceReaper},
    Executor, FenceOutcome,
};
use shaula_store::http_state::SqliteStateBackend;
use std::path::Path;

/// Restart reaper for a crash after terminal commit but before local cleanup.
/// A durable receipt alone is not authority to race a still-live worker tree.
pub(crate) async fn reap(
    backend: &SqliteStateBackend,
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
