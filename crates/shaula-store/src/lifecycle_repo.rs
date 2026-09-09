//! Runner lifecycle ledger persistence: generations and their mutating
//! operation records (the fenced apply-start boundary).

use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use crate::entities::fleet::fleets;
use crate::entities::lifecycle::{runner_generations, runner_operations};
use crate::store::{Store, StoreError, StoreResult};

#[path = "lifecycle_sessions.rs"]
mod sessions;

impl Store {
    pub(crate) async fn generation_insert(
        &self,
        record: shaula_core::registry::GenerationRecord,
    ) -> StoreResult<()> {
        let tx = self.begin().await?;
        Self::generation_insert_on(&tx, record).await?;
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn generation_insert_on<C: sea_orm::ConnectionTrait>(
        connection: &C,
        record: shaula_core::registry::GenerationRecord,
    ) -> StoreResult<()> {
        let (
            id,
            fleet_key,
            runner_name,
            generation_name,
            fleet_revision,
            inputs_digest,
            workspace_path,
            now,
        ) = (
            &record.id,
            &record.fleet_key,
            &record.runner_name,
            &record.generation_name,
            record.fleet_revision,
            &record.inputs_digest,
            &record.workspace_path,
            record.created_at,
        );
        let template = (
            record.template_profile_key.clone(),
            record.template_revision,
            record.template_artifact_digest.clone(),
            record.attestation_id.clone(),
        );
        let row = runner_generations::ActiveModel {
            id: Set(id.to_string()),
            fleet_key: Set(fleet_key.to_string()),
            runner_name: Set(runner_name.to_string()),
            generation_name: Set(generation_name.to_string()),
            fleet_revision: Set(fleet_revision),
            template_profile_key: Set(template.0),
            template_revision: Set(template.1),
            template_artifact_digest: Set(template.2),
            attestation_id: Set(template.3),
            inputs_digest: Set(inputs_digest.to_string()),
            state: Set("CreatePending".to_string()),
            subphase: Set(None),
            jit_phase: Set(None),
            github_runner_id: Set(None),
            workspace_path: Set(workspace_path.to_string()),
            shaula_result_json: Set(None),
            shaula_result_digest: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        };
        runner_generations::Entity::insert(row)
            .exec(connection)
            .await?;
        Self::jobs_snapshot_generation(connection, id, fleet_key, fleet_revision).await?;
        Ok(())
    }

    pub(crate) async fn generation_get(
        &self,
        id: &str,
    ) -> StoreResult<Option<runner_generations::Model>> {
        Ok(runner_generations::Entity::find_by_id(id.to_string())
            .one(self.connection())
            .await?)
    }

    pub(crate) async fn generations_for_fleet(
        &self,
        fleet_key: &str,
    ) -> StoreResult<Vec<runner_generations::Model>> {
        Ok(runner_generations::Entity::find()
            .filter(runner_generations::Column::FleetKey.eq(fleet_key))
            .order_by_asc(runner_generations::Column::CreatedAt)
            .all(self.connection())
            .await?)
    }

    /// Enforces the lifecycle state machine at the persistence boundary.
    pub(crate) async fn generation_advance(
        &self,
        id: &str,
        next: shaula_core::lifecycle::GenerationState,
        subphase: Option<&str>,
        now: i64,
    ) -> StoreResult<runner_generations::Model> {
        let row = self
            .generation_get(id)
            .await?
            .ok_or_else(|| StoreError::Corrupt(format!("generation {id} missing")))?;
        let current = shaula_core::lifecycle::GenerationState::from_str_repr(&row.state)
            .map_err(StoreError::Corrupt)?;
        shaula_core::lifecycle::advance(current, next)
            .map_err(|e| StoreError::Conflict { resource: e })?;
        let mut updated: runner_generations::ActiveModel = row.into();
        updated.state = Set(next.as_str_repr().to_string());
        updated.subphase = Set(subphase.map(str::to_string));
        updated.updated_at = Set(now);
        Ok(runner_generations::Entity::update(updated)
            .exec(self.connection())
            .await?)
    }

    pub(crate) async fn generation_set_jit(
        &self,
        id: &str,
        jit_phase: &str,
        github_runner_id: Option<i64>,
        now: i64,
    ) -> StoreResult<()> {
        let tx = self.begin().await?;
        let row = runner_generations::Entity::find_by_id(id.to_string())
            .one(&tx)
            .await?
            .ok_or_else(|| StoreError::Corrupt(format!("generation {id} missing")))?;
        if row
            .github_runner_id
            .zip(github_runner_id)
            .is_some_and(|(saved, incoming)| saved != incoming)
        {
            return Err(StoreError::Conflict {
                resource: "generation runner identity changed".into(),
            });
        }
        let mut updated: runner_generations::ActiveModel = row.into();
        updated.jit_phase = Set(Some(jit_phase.to_string()));
        if let Some(runner_id) = github_runner_id {
            updated.github_runner_id = Set(Some(runner_id));
        }
        updated.updated_at = Set(now);
        runner_generations::Entity::update(updated)
            .exec(&tx)
            .await?;
        if let Some(runner_id) = github_runner_id {
            Self::jobs_register_runner_on(&tx, id, runner_id).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn generation_set_result(
        &self,
        id: &str,
        result_json: &str,
        digest: &str,
        now: i64,
    ) -> StoreResult<()> {
        let row = self
            .generation_get(id)
            .await?
            .ok_or_else(|| StoreError::Corrupt(format!("generation {id} missing")))?;
        let mut updated: runner_generations::ActiveModel = row.into();
        updated.shaula_result_json = Set(Some(result_json.to_string()));
        updated.shaula_result_digest = Set(Some(digest.to_string()));
        updated.updated_at = Set(now);
        runner_generations::Entity::update(updated)
            .exec(self.connection())
            .await?;
        Ok(())
    }

    /// The generation's ORIGINAL post-Create state identity
    /// `(lineage, serial)`, stored beside the result envelope (spec 0004
    /// §5). Absent/corrupt identity returns `None` — the caller must
    /// quarantine instead of reconstructing it.
    pub(crate) async fn generation_state_identity(
        &self,
        id: &str,
    ) -> StoreResult<Option<(String, u64)>> {
        let Some(row) = self.generation_get(id).await? else {
            return Ok(None);
        };
        let Some(json) = row.shaula_result_json else {
            return Ok(None);
        };
        let value: serde_json::Value = serde_json::from_str(&json)
            .map_err(|e| StoreError::Corrupt(format!("generation result unreadable: {e}")))?;
        let lineage = value
            .get("state_lineage")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let serial = value.get("state_serial").and_then(|v| v.as_u64());
        Ok(match (lineage, serial) {
            (Some(lineage), Some(serial)) if !lineage.is_empty() => Some((lineage, serial)),
            _ => None,
        })
    }

    /// Whether any Destroy operation row exists for this generation —
    /// the retry-vs-first-attempt discriminator for state-serial checks.
    pub(crate) async fn generation_destroy_attempted(&self, id: &str) -> StoreResult<bool> {
        Ok(runner_operations::Entity::find()
            .filter(runner_operations::Column::GenerationId.eq(id.to_string()))
            .filter(runner_operations::Column::Kind.eq("Destroy".to_string()))
            .one(self.connection())
            .await?
            .is_some())
    }

    // ---- Operations ----

    async fn operation_insert_on<C: sea_orm::ConnectionTrait>(
        conn: &C,
        insert: shaula_core::registry::OperationInsert,
    ) -> StoreResult<()> {
        let now = insert.now;
        let row = runner_operations::ActiveModel {
            id: Set(insert.id),
            generation_id: Set(insert.generation_id),
            kind: Set(insert.kind),
            state: Set(insert.state),
            attempts: Set(0),
            next_retry_at: Set(None),
            saved_plan_path: Set(insert.saved_plan_path),
            saved_plan_digest: Set(insert.saved_plan_digest),
            provenance_json: Set(insert.provenance_json),
            lease_owner: Set(None),
            lease_expires_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        };
        runner_operations::Entity::insert(row).exec(conn).await?;
        Ok(())
    }

    /// Plain (non-fenced) operation insert for NON-apply intents such as
    /// the durable `JitStarting` record (R9-07). Apply operations must go
    /// through [`Self::operation_apply_starting_tx`], which carries the
    /// fleet fence.
    pub(crate) async fn operation_insert(
        &self,
        insert: shaula_core::registry::OperationInsert,
    ) -> StoreResult<()> {
        Self::operation_insert_on(self.connection(), insert).await
    }

    /// The FENCED apply-start record, executed entirely inside the
    /// caller's transaction (spec 0002 §8: the Create claim re-verifies
    /// fleet revision/mutation fence/deletion marker when the durable
    /// intent is written — a DELETE or newer PUT can no longer slip in
    /// between the check and the record). A Destroy is fenced ONLY on
    /// generation existence: decommission/replacement must never block
    /// cleanup, which runs with the generation's original materials.
    pub(crate) async fn operation_apply_starting_tx(
        &self,
        tx: &sea_orm::DatabaseTransaction,
        insert: shaula_core::registry::OperationInsert,
    ) -> StoreResult<Result<(), String>> {
        let Some(generation) = runner_generations::Entity::find_by_id(insert.generation_id.clone())
            .one(tx)
            .await?
        else {
            return Ok(Err("generation record missing".to_string()));
        };
        if insert.kind == "Create" {
            let Some(fleet) = fleets::Entity::find_by_id(generation.fleet_key.clone())
                .one(tx)
                .await?
            else {
                return Ok(Err("fleet head missing".to_string()));
            };
            let deleting = fleet.deletion_marker || fleet.tombstone;
            if deleting || fleet.desired_revision != generation.fleet_revision {
                return Ok(Err(format!(
                    "fleet fence moved: head revision {} vs generation {}, deleting={}",
                    fleet.desired_revision, generation.fleet_revision, deleting
                )));
            }
        }
        Self::operation_insert_on(tx, insert).await?;
        Ok(Ok(()))
    }

    pub(crate) async fn operation_update_state(
        &self,
        id: &str,
        state: &str,
        now: i64,
    ) -> StoreResult<()> {
        let row = runner_operations::Entity::find_by_id(id.to_string())
            .one(self.connection())
            .await?
            .ok_or_else(|| StoreError::Corrupt(format!("operation {id} missing")))?;
        let mut updated: runner_operations::ActiveModel = row.into();
        updated.state = Set(state.to_string());
        updated.attempts = Set(updated.attempts.unwrap() + 1);
        updated.updated_at = Set(now);
        runner_operations::Entity::update(updated)
            .exec(self.connection())
            .await?;
        Ok(())
    }

    pub(crate) async fn operations_open_for_generation(
        &self,
        generation_id: &str,
    ) -> StoreResult<Vec<runner_operations::Model>> {
        Ok(runner_operations::Entity::find()
            .filter(runner_operations::Column::GenerationId.eq(generation_id))
            .filter(runner_operations::Column::State.is_in([
                "Pending",
                "Starting",
                "ApplyStarting",
                "Running",
                "Blocked",
            ]))
            .all(self.connection())
            .await?)
    }

    /// The durable ORIGINAL provenance of the generation's first mutating
    /// apply — the pin a later destroy must re-verify against (spec 0004
    /// section 5, exact provenance).
    pub(crate) async fn operation_provenance_get(
        &self,
        generation_id: &str,
        kind: &str,
    ) -> StoreResult<Option<String>> {
        Ok(runner_operations::Entity::find()
            .filter(runner_operations::Column::GenerationId.eq(generation_id))
            .filter(runner_operations::Column::Kind.eq(kind.to_string()))
            .order_by_desc(runner_operations::Column::CreatedAt)
            .one(self.connection())
            .await?
            .and_then(|op| op.provenance_json))
    }
}
