use super::{sql, unavailable, SqliteStateBackend};
use sea_orm::ConnectionTrait;
use sha2::{Digest, Sha256};
use shaula_core::{
    state_backend::{StateError, StateResult},
    worker::{cleanup::WorkspaceCleanup, CompletionReceipt, ProcessIdentity},
};
use std::path::Path;

pub struct CompletedWorkspace {
    pub cleanup: WorkspaceCleanup,
    pub process: Option<ProcessIdentity>,
}

impl SqliteStateBackend {
    /// One row at a time bounds protected input reads, even after a long outage.
    pub async fn next_completed_workspace(
        &self,
        after: &str,
        artifact_root: &Path,
    ) -> StateResult<Option<CompletedWorkspace>> {
        let Some(row) = self.store.connection().query_one(sql(
            "SELECT g.id, g.workspace_path, g.template_artifact_digest, w.protected_input, w.process_identity, w.completion_receipt FROM lifecycle_workers w JOIN runner_generations g ON g.id = w.generation_id WHERE w.phase = 'completed' AND g.state = 'Destroyed' AND w.workspace_reaped = 0 AND g.id > ? ORDER BY g.id LIMIT 1",
            vec![after.into()],
        )).await.map_err(unavailable)? else { return Ok(None); };
        let digest: String = row
            .try_get("", "template_artifact_digest")
            .map_err(unavailable)?;
        let receipt: String = row.try_get("", "completion_receipt").map_err(unavailable)?;
        let process: Option<String> = row.try_get("", "process_identity").map_err(unavailable)?;
        let input: Option<Vec<u8>> = row.try_get("", "protected_input").map_err(unavailable)?;
        let receipt: CompletionReceipt =
            serde_json::from_str(&receipt).map_err(|_| StateError::Invalid)?;
        if receipt.claim.generation_id.to_string()
            != row.try_get::<String>("", "id").map_err(unavailable)?
        {
            return Err(StateError::Conflict);
        }
        Ok(Some(CompletedWorkspace {
            process: process
                .map(|p| serde_json::from_str(&p).map_err(|_| StateError::Invalid))
                .transpose()?,
            cleanup: WorkspaceCleanup {
                receipt,
                workspace: row
                    .try_get::<String>("", "workspace_path")
                    .map_err(unavailable)?
                    .into(),
                artifact: shaula_core::artifact_layout::artifact_dir(artifact_root, &digest)
                    .ok_or(StateError::Invalid)?,
                artifact_digest: digest,
                retained_input_digest: input
                    .map(|bytes| format!("sha256:{}", hex::encode(Sha256::digest(bytes)))),
            },
        }))
    }

    pub async fn workspace_reaped(&self, receipt: &CompletionReceipt) -> StateResult<()> {
        let changed = self.store.connection().execute(sql(
            "UPDATE lifecycle_workers SET workspace_reaped = 1 WHERE generation_id = ? AND phase = 'completed' AND completion_request = ?",
            vec![receipt.claim.generation_id.to_string().into(), receipt.request_id.to_string().into()],
        )).await.map_err(unavailable)?;
        super::exactly_one(changed.rows_affected())
    }
}
