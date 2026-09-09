//! One-way authorization for a Create's host-owned container bootstrap.
use crate::{Store, StoreResult};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use shaula_core::{plan::PlanIntent, ports::PlanProvenance};

impl Store {
    pub(crate) async fn operation_bootstrap_starting(
        &self,
        provenance: &PlanProvenance,
        now: i64,
    ) -> StoreResult<bool> {
        if provenance.intent != PlanIntent::Create {
            return Ok(false);
        }
        let json = serde_json::to_string(provenance)
            .map_err(|_| crate::StoreError::Corrupt("bootstrap provenance invalid".into()))?;
        let changed = self.connection().execute(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "UPDATE runner_operations SET state='BootstrapStarting',updated_at=?
             WHERE id=? AND generation_id=? AND kind='Create' AND state='ApplyStarting'
             AND provenance_json=?
             AND EXISTS (SELECT 1 FROM runner_generations g JOIN fleets f ON f.key=g.fleet_key
                 WHERE g.id=runner_operations.generation_id AND g.state='Creating'
                 AND f.desired_revision=g.fleet_revision AND f.deletion_marker=0 AND f.tombstone=0)",
            [now.into(), provenance.attempt_id.clone().into(), provenance.generation_id.clone().into(), json.into()],
        )).await?;
        Ok(changed.rows_affected() == 1)
    }
}

#[cfg(test)]
#[path = "lifecycle_bootstrap_tests.rs"]
mod tests;
