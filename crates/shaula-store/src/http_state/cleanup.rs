use super::{unavailable, workers, SqliteStateBackend};
use crate::entities::lifecycle::runner_generations as generations;
use crate::query_ext::{same, JoinOn};
use sea_orm::{
    sea_query::Expr, ColumnTrait, EntityTrait, FromQueryResult, QueryFilter, QueryOrder,
    QuerySelect,
};
use sha2::{Digest, Sha256};
use shaula_core::{
    state_backend::{StateError, StateResult},
    worker::{cleanup::WorkspaceCleanup, CompletionReceipt},
};
use std::path::Path;

pub use shaula_core::worker::recovery::CompletedWorkspace;

#[derive(FromQueryResult)]
struct Completed {
    id: String,
    workspace_path: String,
    template_artifact_digest: String,
    protected_input: Option<Vec<u8>>,
    process_identity: Option<String>,
    completion_receipt: String,
}

impl SqliteStateBackend {
    /// One row at a time bounds protected input reads, even after a long outage.
    pub async fn next_completed_workspace(
        &self,
        after: &str,
        artifact_root: &Path,
    ) -> StateResult<Option<CompletedWorkspace>> {
        let Some(row) = generations::Entity::find()
            .select_only()
            .columns([
                generations::Column::Id,
                generations::Column::WorkspacePath,
                generations::Column::TemplateArtifactDigest,
            ])
            .columns([
                workers::Column::ProtectedInput,
                workers::Column::ProcessIdentity,
                workers::Column::CompletionReceipt,
            ])
            .inner_join_on(
                workers::Entity,
                same(generations::Column::Id, workers::Column::GenerationId),
            )
            .filter(workers::Column::Phase.eq("completed"))
            .filter(generations::Column::State.eq("Destroyed"))
            .filter(workers::Column::WorkspaceReaped.eq(false))
            .filter(generations::Column::Id.gt(after))
            .order_by_asc(generations::Column::Id)
            .limit(1)
            .into_model::<Completed>()
            .one(self.store.connection())
            .await
            .map_err(unavailable)?
        else {
            return Ok(None);
        };
        let digest = row.template_artifact_digest;
        let receipt: CompletionReceipt =
            serde_json::from_str(&row.completion_receipt).map_err(|_| StateError::Invalid)?;
        if receipt.claim.generation_id.to_string() != row.id {
            return Err(StateError::Conflict);
        }
        Ok(Some(CompletedWorkspace {
            process: row
                .process_identity
                .map(|p| serde_json::from_str(&p).map_err(|_| StateError::Invalid))
                .transpose()?,
            cleanup: WorkspaceCleanup {
                receipt,
                workspace: row.workspace_path.into(),
                artifact: shaula_core::artifact_layout::artifact_dir(artifact_root, &digest)
                    .ok_or(StateError::Invalid)?,
                artifact_digest: digest,
                retained_input_digest: row
                    .protected_input
                    .map(|bytes| format!("sha256:{}", hex::encode(Sha256::digest(bytes)))),
            },
        }))
    }

    pub async fn workspace_reaped(&self, receipt: &CompletionReceipt) -> StateResult<()> {
        let changed = workers::Entity::update_many()
            .col_expr(workers::Column::WorkspaceReaped, Expr::value(true))
            .filter(workers::Column::GenerationId.eq(receipt.claim.generation_id.to_string()))
            .filter(workers::Column::Phase.eq("completed"))
            .filter(workers::Column::CompletionRequest.eq(receipt.request_id.to_string()))
            .exec(self.store.connection())
            .await
            .map_err(unavailable)?;
        super::exactly_one(changed.rows_affected)
    }
}
