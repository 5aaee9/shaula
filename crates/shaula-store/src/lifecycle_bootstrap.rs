//! One-way authorization for a Create's host-owned container bootstrap.
use crate::entities::{
    fleet::fleets,
    lifecycle::{runner_generations as generations, runner_operations as operations},
};
use crate::query_ext::{col, probe, same};
use crate::{Store, StoreResult};
use sea_orm::{sea_query::Expr, ColumnTrait, EntityTrait, QueryFilter};
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
        let tx = self.begin().await?;
        let current = probe(generations::Entity)
            .inner_join(
                fleets::Entity,
                same(fleets::Column::Key, generations::Column::FleetKey),
            )
            .and_where(same(
                generations::Column::Id,
                operations::Column::GenerationId,
            ))
            .and_where(col(generations::Column::State).eq("Creating"))
            .and_where(same(
                fleets::Column::DesiredRevision,
                generations::Column::FleetRevision,
            ))
            .and_where(col(fleets::Column::DeletionMarker).eq(false))
            .and_where(col(fleets::Column::Tombstone).eq(false))
            .to_owned();
        let changed = operations::Entity::update_many()
            .col_expr(operations::Column::State, Expr::value("BootstrapStarting"))
            .col_expr(operations::Column::UpdatedAt, Expr::value(now))
            .filter(operations::Column::Id.eq(provenance.attempt_id.as_str()))
            .filter(operations::Column::GenerationId.eq(provenance.generation_id.as_str()))
            .filter(operations::Column::Kind.eq("Create"))
            .filter(operations::Column::State.eq("ApplyStarting"))
            .filter(operations::Column::ProvenanceJson.eq(json))
            .filter(Expr::exists(current))
            .exec(&tx)
            .await?;
        if changed.rows_affected == 1 {
            crate::http_state::SqliteStateBackend::worker_bootstrap_on(&tx, provenance)
                .await
                .map_err(|_| {
                    crate::StoreError::Corrupt("worker bootstrap authority unavailable".into())
                })?;
            tx.commit().await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[cfg(test)]
#[path = "lifecycle_bootstrap_tests.rs"]
mod tests;
