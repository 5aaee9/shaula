//! Offline legacy classification and one-way activation. Filesystem evidence
//! is checked by the maintenance caller while it holds the data-directory lock.
use super::{sql, unavailable, SqliteStateBackend};
use sea_orm::{ConnectionTrait, FromQueryResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use shaula_core::state_backend::{StateDocument, StateError, StateResult};
use shaula_core::worker::WorkerAdmission;
use uuid::Uuid;

#[derive(FromQueryResult, Serialize)]
pub struct LegacyGeneration {
    pub id: String,
    pub fleet_key: String,
    pub workspace_path: String,
    pub template_artifact_digest: String,
    pub inputs_digest: String,
    pub state: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub effects: String,
    pub authority: String,
    pub create_result: Option<String>,
    pub resources_destroyed_at: Option<i64>,
}

impl LegacyGeneration {
    /// Imported bytes must be anchored to the daemon's retained successful
    /// Create, rather than certify their own ownership or completeness.
    pub fn state_matches_original(&self, state: &StateDocument) -> bool {
        super::create_proof::matches(self.create_result.as_deref(), state)
    }
    pub fn commitment(&self) -> StateResult<String> {
        Ok(format!(
            "sha256:{}",
            hex::encode(Sha256::digest(
                serde_json::to_vec(self).map_err(|_| StateError::Invalid)?
            ))
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationClassification {
    Imported,
    Quarantined,
}

impl SqliteStateBackend {
    pub async fn deployment_identity(&self) -> StateResult<String> {
        self.store
            .connection()
            .query_one(sql(
                "SELECT deployment_id FROM lifecycle_deployment WHERE singleton = 1",
                vec![],
            ))
            .await
            .map_err(unavailable)?
            .ok_or(StateError::Unavailable)?
            .try_get("", "deployment_id")
            .map_err(unavailable)
    }
    pub async fn import_receipt(&self, id: &str, commitment: &str) -> StateResult<bool> {
        let saved = self
            .store
            .connection()
            .query_one(sql(
                "SELECT commitment FROM lifecycle_imports WHERE generation_id = ?",
                vec![id.into()],
            ))
            .await
            .map_err(unavailable)?;
        match saved {
            Some(saved)
                if saved
                    .try_get::<String>("", "commitment")
                    .map_err(unavailable)?
                    == commitment =>
            {
                Ok(true)
            }
            Some(_) => Err(StateError::Conflict),
            None => Ok(false),
        }
    }
    pub async fn legacy_generations(&self) -> StateResult<Vec<LegacyGeneration>> {
        let rows = self.store.connection().query_all(sql(
            "SELECT g.id, g.fleet_key, g.workspace_path, g.template_artifact_digest, g.inputs_digest, g.shaula_result_json AS create_result, g.resources_destroyed_at,
                g.state, g.created_at, g.updated_at, json_array(g.runner_name,g.generation_name,g.fleet_revision,g.template_profile_key,g.template_revision,g.attestation_id,g.pool_member_key,g.subphase,g.jit_phase,g.github_runner_id,g.shaula_result_json,g.shaula_result_digest,g.provisioned_at,g.expiry_requested_at,g.resources_destroyed_at,(SELECT json_array(f.runner_id,f.runner_uuid,f.created_at,f.updated_at) FROM forgejo_runner_identities f WHERE f.generation_id = g.id)) AS authority,
                (SELECT json_group_array(json_object('id',o.id,'kind',o.kind,'state',o.state,'provenance',o.provenance_json,'updated_at',o.updated_at,'attempts',o.attempts,'plan',o.saved_plan_digest,'path',o.saved_plan_path,'owner',o.lease_owner,'lease',o.lease_expires_at,'retry',o.next_retry_at,'created_at',o.created_at)) FROM (SELECT * FROM runner_operations WHERE generation_id = g.id ORDER BY id) o) AS effects FROM runner_generations g
             WHERE g.state != 'Destroyed'
               AND NOT EXISTS (SELECT 1 FROM lifecycle_workers w WHERE w.generation_id = g.id)
               AND NOT EXISTS (SELECT 1 FROM lifecycle_imports i WHERE i.generation_id = g.id)
             ORDER BY g.id", vec![],
        )).await.map_err(unavailable)?;
        rows.iter()
            .map(|row| LegacyGeneration::from_query_result(row, "").map_err(unavailable))
            .collect()
    }

    pub async fn activated(&self) -> StateResult<bool> {
        let row = self.store.connection().query_one(sql(
            "SELECT activated FROM lifecycle_deployment WHERE singleton = 1 AND format_version = 1", vec![],
        )).await.map_err(unavailable)?.ok_or(StateError::Unavailable)?;
        row.try_get("", "activated").map_err(unavailable)
    }

    pub async fn activate(&self) -> StateResult<()> {
        let tx = self.writer("").await?;
        let unclassified = tx
            .query_one(sql(
                "SELECT COUNT(*) AS count FROM runner_generations g WHERE g.state != 'Destroyed'
             AND NOT EXISTS (SELECT 1 FROM lifecycle_workers w WHERE w.generation_id = g.id)
             AND NOT EXISTS (SELECT 1 FROM lifecycle_imports i WHERE i.generation_id = g.id)",
                vec![],
            ))
            .await
            .map_err(unavailable)?
            .ok_or(StateError::Unavailable)?;
        if unclassified
            .try_get::<i64>("", "count")
            .map_err(unavailable)?
            != 0
        {
            tx.execute(sql(
                "UPDATE lifecycle_deployment SET migration_required = 1 WHERE singleton = 1",
                vec![],
            ))
            .await
            .map_err(unavailable)?;
            tx.commit().await.map_err(unavailable)?;
            return Err(StateError::Conflict);
        }
        tx.execute(sql("UPDATE lifecycle_deployment SET activated = 1, migration_required = 0 WHERE singleton = 1 AND format_version = 1", vec![])).await.map_err(unavailable)?;
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
        if let Some(saved) = tx
            .query_one(sql(
                "SELECT commitment, classification, receipt FROM lifecycle_imports WHERE generation_id = ?",
                vec![generation.id.clone().into()],
            ))
            .await
            .map_err(unavailable)?
        {
            let expected = match classification {
                MigrationClassification::Imported => "imported",
                MigrationClassification::Quarantined => "quarantined",
            };
            if saved
                .try_get::<String>("", "commitment")
                .map_err(unavailable)?
                == commitment
                && saved
                    .try_get::<String>("", "classification")
                    .map_err(unavailable)?
                    == expected
                && serde_json::from_str::<serde_json::Value>(&saved.try_get::<String>("", "receipt").map_err(unavailable)?).map_err(|_| StateError::Unavailable)?.get("content").and_then(|v| v.as_str()) == Some(&content)
            {
                return Ok(());
            }
            return Err(StateError::Conflict);
        }
        let row = tx.query_one(sql(
            "SELECT g.id, g.fleet_key, g.workspace_path, g.template_artifact_digest, g.inputs_digest, g.shaula_result_json AS create_result, g.resources_destroyed_at, g.state, g.created_at, g.updated_at, json_array(g.runner_name,g.generation_name,g.fleet_revision,g.template_profile_key,g.template_revision,g.attestation_id,g.pool_member_key,g.subphase,g.jit_phase,g.github_runner_id,g.shaula_result_json,g.shaula_result_digest,g.provisioned_at,g.expiry_requested_at,g.resources_destroyed_at,(SELECT json_array(f.runner_id,f.runner_uuid,f.created_at,f.updated_at) FROM forgejo_runner_identities f WHERE f.generation_id = g.id)) AS authority, (SELECT json_group_array(json_object('id',o.id,'kind',o.kind,'state',o.state,'provenance',o.provenance_json,'updated_at',o.updated_at,'attempts',o.attempts,'plan',o.saved_plan_digest,'path',o.saved_plan_path,'owner',o.lease_owner,'lease',o.lease_expires_at,'retry',o.next_retry_at,'created_at',o.created_at)) FROM (SELECT * FROM runner_operations WHERE generation_id = g.id ORDER BY id) o) AS effects FROM runner_generations g WHERE g.id = ?",
            vec![generation.id.clone().into()],
        )).await.map_err(unavailable)?.ok_or(StateError::Conflict)?;
        let current = LegacyGeneration::from_query_result(&row, "").map_err(unavailable)?;
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
                tx.execute(sql(
                    "INSERT INTO generation_http_state (generation_id, worker_epoch, worker_attempt, capability_hash, revoked, create_started, revision, state_bytes, lineage, serial)
                     VALUES (?, 1, ?, ?, 1, 1, 1, ?, ?, ?)",
                    vec![generation.id.clone().into(), admission.claim.worker_attempt.to_string().into(), admission.state.verifier().into(), state.bytes().to_vec().into(), state.lineage().into(), state.serial().into()],
                )).await.map_err(unavailable)?;
                tx.execute(sql("INSERT INTO lifecycle_workers(generation_id, control_hash, phase, cleanup_only, protected_input) VALUES (?, ?, 'fenced', 1, ?)", vec![generation.id.clone().into(), admission.control.verifier().into(), input.into()])).await.map_err(unavailable)?;
                tx.execute(sql(
                    "UPDATE lifecycle_workers SET resource_state_seen = 1, cleanup_revision = ? WHERE generation_id = ?",
                    vec![cleanup_revision.into(), generation.id.clone().into()],
                ))
                .await
                .map_err(unavailable)?;
                tx.execute(sql(
                    "UPDATE runner_generations SET workspace_path = ? WHERE id = ?",
                    vec![workspace.into(), generation.id.clone().into()],
                ))
                .await
                .map_err(unavailable)?;
                "imported"
            }
            MigrationClassification::Quarantined => {
                tx.execute(sql(
                    "UPDATE runner_generations SET state = 'Quarantined' WHERE id = ?",
                    vec![generation.id.clone().into()],
                ))
                .await
                .map_err(unavailable)?;
                tx.execute(sql(
                    "UPDATE generation_http_state SET revoked = 1 WHERE generation_id = ?",
                    vec![generation.id.clone().into()],
                ))
                .await
                .map_err(unavailable)?;
                "quarantined"
            }
        };
        let receipt = serde_json::json!({"generation_id": generation.id, "commitment": commitment, "classification": classification, "content": content, "format_version": 1}).to_string();
        tx.execute(sql("INSERT INTO lifecycle_imports(generation_id, commitment, classification, receipt) VALUES (?, ?, ?, ?)", vec![generation.id.clone().into(), commitment.into(), classification.into(), receipt.into()])).await.map_err(unavailable)?;
        tx.commit().await.map_err(unavailable)
    }
}

fn valid_digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}
