//! Offline legacy classification and one-way activation. Filesystem evidence
//! is checked by the maintenance caller while it holds the data-directory lock.
use super::{
    deployment, legacy, states, unavailable, workers, LegacyGeneration, SqliteStateBackend,
};
use crate::entities::http_state::lifecycle_imports as imports;
use crate::entities::lifecycle::runner_generations as generations;
use sea_orm::{
    sea_query::Expr, ActiveValue::Set, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use shaula_core::state_backend::{StateDocument, StateError, StateResult};
use shaula_core::worker::WorkerAdmission;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationClassification {
    Imported,
    Quarantined,
}

impl SqliteStateBackend {
    pub async fn deployment_identity(&self) -> StateResult<String> {
        deployment::Entity::find_by_id(1)
            .select_only()
            .column(deployment::Column::DeploymentId)
            .into_tuple::<String>()
            .one(self.store.connection())
            .await
            .map_err(unavailable)?
            .ok_or(StateError::Unavailable)
    }
    pub async fn import_receipt(&self, id: &str, commitment: &str) -> StateResult<bool> {
        let saved = imports::Entity::find_by_id(id)
            .one(self.store.connection())
            .await
            .map_err(unavailable)?;
        match saved {
            Some(saved) if saved.commitment == commitment => Ok(true),
            Some(_) => Err(StateError::Conflict),
            None => Ok(false),
        }
    }
    pub async fn legacy_generations(&self) -> StateResult<Vec<LegacyGeneration>> {
        legacy::select()
            .filter(legacy::unclassified())
            .order_by_asc(generations::Column::Id)
            .into_model::<LegacyGeneration>()
            .all(self.store.connection())
            .await
            .map_err(unavailable)
    }

    pub async fn activated(&self) -> StateResult<bool> {
        deployment::Entity::find_by_id(1)
            .select_only()
            .column(deployment::Column::Activated)
            .filter(deployment::Column::FormatVersion.eq(1))
            .into_tuple::<bool>()
            .one(self.store.connection())
            .await
            .map_err(unavailable)?
            .ok_or(StateError::Unavailable)
    }

    pub async fn activate(&self) -> StateResult<()> {
        let tx = self.writer("").await?;
        let unclassified = generations::Entity::find()
            .filter(legacy::unclassified())
            .count(&tx)
            .await
            .map_err(unavailable)?;
        if unclassified != 0 {
            deployment::Entity::update_many()
                .col_expr(deployment::Column::MigrationRequired, Expr::value(true))
                .filter(deployment::Column::Singleton.eq(1))
                .exec(&tx)
                .await
                .map_err(unavailable)?;
            tx.commit().await.map_err(unavailable)?;
            return Err(StateError::Conflict);
        }
        deployment::Entity::update_many()
            .col_expr(deployment::Column::Activated, Expr::value(true))
            .col_expr(deployment::Column::MigrationRequired, Expr::value(false))
            .filter(deployment::Column::Singleton.eq(1))
            .filter(deployment::Column::FormatVersion.eq(1))
            .exec(&tx)
            .await
            .map_err(unavailable)?;
        tx.commit().await.map_err(unavailable)
    }

    /// Exact original state bytes are imported, never synthesized for legacy.
    /// `commitment` binds both database and filesystem inspect/apply evidence.
    pub async fn import_legacy(
        &self,
        generation: &LegacyGeneration,
        commitment: &str,
        state: Option<StateDocument>,
        classification: MigrationClassification,
        protected_input: Option<Vec<u8>>,
        workspace: Option<&str>,
    ) -> StateResult<()> {
        if !valid_digest(commitment) {
            return Err(StateError::Invalid);
        }
        let tx = self.writer(&generation.id).await?;
        let content = format!("sha256:{}", hex::encode(Sha256::digest(serde_json::to_vec(&serde_json::json!({"state": state.as_ref().map(|state| hex::encode(Sha256::digest(state.bytes()))), "input": protected_input.as_ref().map(|input| hex::encode(Sha256::digest(input))), "workspace": workspace})).map_err(|_| StateError::Invalid)?)));
        if let Some(saved) = imports::Entity::find_by_id(&generation.id)
            .one(&tx)
            .await
            .map_err(unavailable)?
        {
            let expected = match classification {
                MigrationClassification::Imported => "imported",
                MigrationClassification::Quarantined => "quarantined",
            };
            let receipt = serde_json::from_str::<serde_json::Value>(&saved.receipt)
                .map_err(|_| StateError::Unavailable)?;
            if saved.commitment == commitment
                && saved.classification == expected
                && receipt.get("content").and_then(|v| v.as_str()) == Some(&content)
            {
                return Ok(());
            }
            return Err(StateError::Conflict);
        }
        let current = legacy::select()
            .filter(generations::Column::Id.eq(&generation.id))
            .into_model::<LegacyGeneration>()
            .one(&tx)
            .await
            .map_err(unavailable)?
            .ok_or(StateError::Conflict)?;
        if current.commitment()? != generation.commitment()? || current.state == "Destroyed" {
            return Err(StateError::Conflict);
        }
        let classification = match classification {
            MigrationClassification::Imported => {
                let state = state.ok_or(StateError::Invalid)?;
                if !current.state_matches_original(&state) {
                    return Err(StateError::Conflict);
                }
                // The retained successful Create proves resources were fully
                // recorded even when a later completed Destroy left empty state.
                let cleanup_revision = (state.managed_empty()
                    && current.resources_destroyed_at.is_some())
                .then_some(1_i64);
                let input = protected_input.ok_or(StateError::Invalid)?;
                let workspace = workspace.ok_or(StateError::Invalid)?;
                if input.len() > shaula_core::state_backend::MAX_STATE_BYTES
                    || workspace == generation.workspace_path
                {
                    return Err(StateError::Invalid);
                }
                let admission = WorkerAdmission::new(
                    Uuid::parse_str(&generation.id).map_err(|_| StateError::Invalid)?,
                );
                // No live authority is issued offline. Recovery replaces this
                // revoked epoch after proving the prior executor tree fenced.
                states::Entity::insert(states::ActiveModel {
                    generation_id: Set(generation.id.clone()),
                    worker_epoch: Set(1),
                    worker_attempt: Set(admission.claim.worker_attempt.to_string()),
                    capability_hash: Set(admission.state.verifier()),
                    revoked: Set(true),
                    create_started: Set(true),
                    revision: Set(1),
                    state_bytes: Set(Some(state.bytes().to_vec())),
                    lineage: Set(Some(state.lineage().to_owned())),
                    serial: Set(Some(state.serial())),
                    ..Default::default()
                })
                .exec_without_returning(&tx)
                .await
                .map_err(unavailable)?;
                workers::Entity::insert(workers::ActiveModel {
                    generation_id: Set(generation.id.clone()),
                    control_hash: Set(admission.control.verifier()),
                    phase: Set("fenced".into()),
                    cleanup_only: Set(true),
                    protected_input: Set(Some(input)),
                    resource_state_seen: Set(true),
                    cleanup_revision: Set(cleanup_revision),
                    ..Default::default()
                })
                .exec_without_returning(&tx)
                .await
                .map_err(unavailable)?;
                generations::Entity::update_many()
                    .col_expr(generations::Column::WorkspacePath, Expr::value(workspace))
                    .filter(generations::Column::Id.eq(&generation.id))
                    .exec(&tx)
                    .await
                    .map_err(unavailable)?;
                "imported"
            }
            MigrationClassification::Quarantined => {
                generations::Entity::update_many()
                    .col_expr(generations::Column::State, Expr::value("Quarantined"))
                    .filter(generations::Column::Id.eq(&generation.id))
                    .exec(&tx)
                    .await
                    .map_err(unavailable)?;
                states::Entity::update_many()
                    .col_expr(states::Column::Revoked, Expr::value(true))
                    .filter(states::Column::GenerationId.eq(&generation.id))
                    .exec(&tx)
                    .await
                    .map_err(unavailable)?;
                "quarantined"
            }
        };
        let receipt = serde_json::json!({"generation_id": generation.id, "commitment": commitment, "classification": classification, "content": content, "format_version": 1}).to_string();
        imports::Entity::insert(imports::ActiveModel {
            generation_id: Set(generation.id.clone()),
            commitment: Set(commitment.into()),
            classification: Set(classification.into()),
            receipt: Set(receipt),
        })
        .exec_without_returning(&tx)
        .await
        .map_err(unavailable)?;
        tx.commit().await.map_err(unavailable)
    }
}

fn valid_digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}
