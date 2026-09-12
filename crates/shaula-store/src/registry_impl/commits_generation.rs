//! Generation-level commit implementations (spec 0028 operator finalize),
//! split to keep files within 400 lines (AGENTS.md).

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use shaula_core::error::CoreResult;
use shaula_core::registry::MutationError;

use crate::entities::lifecycle::runner_generations;
use crate::store::{Store, StoreError};

use super::{core_err, SqliteControlPlane};

impl SqliteControlPlane {
    /// Operator finalization of a Quarantined generation: CAS the state
    /// column inside the transaction, append the Finalize operation row,
    /// the audit fact and the optional idempotency record. No remote
    /// effect runs — the ledger transition is the only durable fact.
    pub(crate) async fn commit_generation_finalize_impl(
        &self,
        generation_id: &str,
        operation_id: &str,
        actor: &str,
        reason: &str,
        idempotency: Option<shaula_core::registry::IdempotencyInsert>,
        now: i64,
    ) -> CoreResult<Result<(), MutationError>> {
        let tx = self.store.begin().await.map_err(core_err)?;
        let row = runner_generations::Entity::find()
            .filter(runner_generations::Column::Id.eq(generation_id))
            .one(&tx)
            .await
            .map_err(|e| core_err(StoreError::from(e)))?;
        let Some(row) = row else {
            let _ = tx.rollback().await;
            return Ok(Err(MutationError::NotFound));
        };
        if row.state != shaula_core::lifecycle::GenerationState::Quarantined.as_str_repr() {
            let _ = tx.rollback().await;
            return Ok(Err(MutationError::Conflict {
                summary: format!(
                    "generation is {}, not Quarantined; finalize only terminates quarantined rows",
                    row.state
                ),
            }));
        }
        // transition_allowed(Quarantined, Destroyed) is the sole edge out
        // of quarantine; the CAS write below races with nothing else that
        // can move a Quarantined row (supervisor only ever ENTERS it).
        // CAS: the state column must still be Quarantined at write time.
        let cas = runner_generations::Entity::update_many()
            .col_expr(
                runner_generations::Column::State,
                sea_orm::sea_query::Expr::value(
                    shaula_core::lifecycle::GenerationState::Destroyed.as_str_repr(),
                ),
            )
            .col_expr(
                runner_generations::Column::UpdatedAt,
                sea_orm::sea_query::Expr::value(now),
            )
            .filter(runner_generations::Column::Id.eq(generation_id))
            .filter(
                runner_generations::Column::State
                    .eq(shaula_core::lifecycle::GenerationState::Quarantined.as_str_repr()),
            )
            .exec(&tx)
            .await
            .map_err(|e| core_err(StoreError::from(e)))?;
        if cas.rows_affected != 1 {
            let _ = tx.rollback().await;
            return Ok(Err(MutationError::Conflict {
                summary: "generation left Quarantined between admission and commit".into(),
            }));
        }

        Store::operation_insert_on(
            &tx,
            shaula_core::registry::OperationInsert {
                id: operation_id.to_string(),
                generation_id: generation_id.to_string(),
                kind: "Finalize".to_string(),
                state: "Succeeded".to_string(),
                provenance_json: Some(
                    serde_json::json!({
                        "actor": actor,
                        "reason": reason,
                    })
                    .to_string(),
                ),
                saved_plan_path: None,
                saved_plan_digest: None,
                now,
            },
        )
        .await
        .map_err(core_err)?;

        self.store
            .audit_append(
                &tx,
                shaula_core::registry::AuditAppend {
                    resource_kind: "runner_generation".into(),
                    action: "finalize".into(),
                    actor: actor.to_string(),
                    resource_key: generation_id.to_string(),
                    revision: None,
                    outcome: "accepted".into(),
                    detail_json: Some(serde_json::json!({"reason": reason}).to_string()),
                    now,
                },
            )
            .await
            .map_err(core_err)?;

        if let Some(idem) = idempotency {
            self.store
                .idempotency_store(&tx, idem)
                .await
                .map_err(core_err)?;
        }
        tx.commit()
            .await
            .map_err(|e| core_err(StoreError::from(e)))?;
        Ok(Ok(()))
    }
}
