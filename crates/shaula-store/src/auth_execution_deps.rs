//! A Fleet's live execution dependencies: each still-referenced auth
//! revision with the target it serves and its retained context.
use super::target_from_spec;
use crate::entities::auth::fleet_session_auth;
use crate::entities::fleet::{fleet_auth_handoffs as handoffs, fleet_revisions, fleets};
use crate::entities::lifecycle::{
    fleet_sessions, runner_generations as generations, runner_operations as operations,
};
use crate::entities::listener::{
    listener_acquisitions as acquisitions, listener_messages as messages,
};
use crate::query_ext::{col, probe, same, JoinOn};
use crate::store::{Store, StoreError, StoreResult};
use sea_orm::{
    sea_query::{Expr, SimpleExpr},
    ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, QuerySelect,
};
use shaula_core::auth_context::ResolvedAuthContext;

pub(crate) struct ExecutionDependency {
    pub target_json: String,
    pub reference: (String, i64),
    pub context: Option<ResolvedAuthContext>,
}

/// Operation states that still hold a Destroyed Generation's auth reference.
const ACTIVE_OPERATIONS: [&str; 6] = [
    "Pending",
    "Starting",
    "ApplyStarting",
    "BootstrapStarting",
    "Running",
    "Blocked",
];

/// `fleet_revisions` row a Fleet's desired head points at.
fn desired_head() -> SimpleExpr {
    same(fleet_revisions::Column::FleetKey, fleets::Column::Key).and(same(
        fleet_revisions::Column::Revision,
        fleets::Column::DesiredRevision,
    ))
}

impl Store {
    /// Auth references a Fleet still holds outside its Generations: the
    /// observed handoff, the live session, and messages with open acquisitions.
    async fn auth_fleet_refs_tx(
        &self,
        tx: &DatabaseTransaction,
        fleet: &str,
    ) -> StoreResult<Vec<((String, i64), String)>> {
        let observed = handoffs::Entity::find()
            .select_only()
            .columns([
                handoffs::Column::ObservedProfileKey,
                handoffs::Column::ObservedRevision,
            ])
            .column(fleet_revisions::Column::SpecJson)
            .inner_join_on(
                fleets::Entity,
                same(fleets::Column::Key, handoffs::Column::FleetKey),
            )
            .inner_join_on(fleet_revisions::Entity, desired_head())
            .filter(handoffs::Column::FleetKey.eq(fleet))
            .filter(handoffs::Column::ObservedProfileKey.is_not_null())
            .into_tuple::<(String, i64, String)>()
            .all(tx)
            .await?;
        let session = fleet_session_auth::Entity::find()
            .select_only()
            .columns([
                fleet_session_auth::Column::ProfileKey,
                fleet_session_auth::Column::Revision,
            ])
            .column(fleet_revisions::Column::SpecJson)
            .inner_join_on(
                fleet_sessions::Entity,
                same(
                    fleet_sessions::Column::FleetKey,
                    fleet_session_auth::Column::FleetKey,
                ),
            )
            .inner_join_on(
                fleets::Entity,
                same(fleets::Column::Key, fleet_session_auth::Column::FleetKey),
            )
            .inner_join_on(fleet_revisions::Entity, desired_head())
            .filter(fleet_session_auth::Column::FleetKey.eq(fleet))
            .into_tuple::<(String, i64, String)>()
            .all(tx)
            .await?;
        let open = probe(acquisitions::Entity)
            .and_where(same(
                acquisitions::Column::FleetKey,
                messages::Column::FleetKey,
            ))
            .and_where(same(acquisitions::Column::Epoch, messages::Column::Epoch))
            .and_where(same(
                acquisitions::Column::MessageId,
                messages::Column::MessageId,
            ))
            .and_where(col(acquisitions::Column::State).is_in([
                "Pending",
                "AcquireStarting",
                "Uncertain",
            ]))
            .to_owned();
        let pending = messages::Entity::find()
            .select_only()
            .columns([messages::Column::ProfileKey, messages::Column::AuthRevision])
            .column(fleet_revisions::Column::SpecJson)
            .inner_join_on(
                fleet_revisions::Entity,
                same(
                    fleet_revisions::Column::FleetKey,
                    messages::Column::FleetKey,
                )
                .and(same(
                    fleet_revisions::Column::Revision,
                    messages::Column::FleetRevision,
                )),
            )
            .filter(messages::Column::FleetKey.eq(fleet))
            .filter(Expr::exists(open))
            .into_tuple::<(String, i64, String)>()
            .all(tx)
            .await?;
        Ok(observed
            .into_iter()
            .chain(session)
            .chain(pending)
            .map(|(key, revision, spec)| ((key, revision), spec))
            .collect())
    }

    pub(crate) async fn auth_execution_dependencies_tx(
        &self,
        tx: &DatabaseTransaction,
        fleet: &str,
        profile: Option<&str>,
    ) -> StoreResult<Vec<ExecutionDependency>> {
        let mut references = self.auth_fleet_refs_tx(tx, fleet).await?;
        let running = probe(operations::Entity)
            .and_where(same(
                operations::Column::GenerationId,
                generations::Column::Id,
            ))
            .and_where(col(operations::Column::State).is_in(ACTIVE_OPERATIONS))
            .to_owned();
        let live = generations::Entity::find()
            .select_only()
            .column(generations::Column::Id)
            .column(fleet_revisions::Column::SpecJson)
            .inner_join_on(
                fleet_revisions::Entity,
                same(
                    fleet_revisions::Column::FleetKey,
                    generations::Column::FleetKey,
                )
                .and(same(
                    fleet_revisions::Column::Revision,
                    generations::Column::FleetRevision,
                )),
            )
            .filter(generations::Column::FleetKey.eq(fleet))
            .filter(
                col(generations::Column::State)
                    .ne("Destroyed")
                    .or(Expr::exists(running)),
            )
            .into_tuple::<(String, String)>()
            .all(tx)
            .await?;
        for (id, spec) in live {
            let reference = self
                .auth_generation_ref_tx(tx, &id)
                .await?
                .ok_or_else(|| StoreError::Corrupt("generation auth reference missing".into()))?;
            references.push((reference, spec));
        }
        let mut result = Vec::new();
        for (reference, spec) in references {
            if profile.is_some_and(|key| key != reference.0) {
                continue;
            }
            let revision = self
                .auth_revision_get_tx(tx, &reference.0, reference.1)
                .await?
                .ok_or_else(|| StoreError::Corrupt("execution auth revision missing".into()))?;
            // Inventory must retain unsupported references for recovery. It
            // never loads their policy, credential or context as authority.
            let (target_json, context) = if revision.schema_version == 2
                && revision.kind == "github_app"
            {
                let context = self
                    .auth_execution_context_tx(tx, fleet, &reference.0, reference.1)
                    .await?
                    .ok_or_else(|| StoreError::Corrupt("v2 execution context missing".into()))?;
                (
                    serde_json::to_string(&context.target)
                        .map_err(|e| StoreError::Corrupt(e.to_string()))?,
                    Some(context),
                )
            } else {
                (target_from_spec(&spec)?, None)
            };
            result.push(ExecutionDependency {
                target_json,
                reference,
                context,
            });
        }
        Ok(result)
    }

    pub(crate) async fn auth_execution_refs(&self, fleet: &str) -> StoreResult<Vec<(String, i64)>> {
        let tx = self.begin().await?;
        let mut refs: Vec<_> = self
            .auth_execution_dependencies_tx(&tx, fleet, None)
            .await?
            .into_iter()
            .map(|d| d.reference)
            .collect();
        refs.sort();
        refs.dedup();
        tx.commit().await?;
        Ok(refs)
    }
}
