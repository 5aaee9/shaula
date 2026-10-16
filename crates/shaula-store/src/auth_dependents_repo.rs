//! Coverage includes desired admission and every still-live execution target.
use crate::entities::fleet::{fleet_revisions, fleets};
use crate::query_ext::{col, same, scalar, JoinOn};
use crate::store::{Store, StoreResult};
use sea_orm::{
    sea_query::{Alias, Expr, Query},
    DatabaseTransaction, EntityTrait, QueryFilter,
};
use shaula_core::registry::AuthDependentTarget;

impl Store {
    pub(crate) async fn auth_live_dependents_tx(
        &self,
        tx: &DatabaseTransaction,
        key: &str,
    ) -> StoreResult<Vec<AuthDependentTarget>> {
        let mut dependents = Vec::new();
        // One read for every fleet's latest revision at or below its head;
        // (fleet_key, revision) is unique, so each fleet maps to one row.
        let mut latest_revisions: std::collections::HashMap<_, _> = fleet_revisions::Entity::find()
            .inner_join_on(
                fleets::Entity,
                same(fleets::Column::Key, fleet_revisions::Column::FleetKey),
            )
            .filter(col(fleet_revisions::Column::Revision).eq(scalar(latest_at_or_below_head())))
            .all(tx)
            .await?
            .into_iter()
            .map(|revision| (revision.fleet_key.clone(), revision))
            .collect();
        for fleet in fleets::Entity::find().all(tx).await? {
            let Some(latest) = latest_revisions.remove(&fleet.key) else {
                continue;
            };
            // Pool (Forgejo/GitLab) DELETE advances the head without a synthetic spec revision.
            // Keep its scope/credential dependency until cleanup is complete.
            if latest.revision != fleet.desired_revision
                && !serde_json::from_str::<serde_json::Value>(&latest.spec_json)
                    .is_ok_and(|spec| matches!(spec["kind"].as_str(), Some("forgejo" | "gitlab")))
            {
                continue;
            }
            let mut targets = std::collections::BTreeMap::<String, AuthDependentTarget>::new();
            let base = |target_json: String| AuthDependentTarget {
                fleet_key: fleet.key.clone(),
                fleet_phase: fleet.phase.clone(),
                target_json,
                incarnation: fleet.incarnation.clone(),
                revision: fleet.desired_revision,
                fence: fleet.mutation_fence,
                retained_contexts: Vec::new(),
                retained_refs: Vec::new(),
            };
            if !fleet.tombstone && latest.auth_desired_profile_key == key {
                let target = crate::auth_execution_repo::target_from_spec(&latest.spec_json)?;
                targets.insert(target.clone(), base(target));
            }
            for execution in self
                .auth_execution_dependencies_tx(tx, &fleet.key, Some(key))
                .await?
            {
                if execution.reference.0 != key {
                    continue;
                }
                let entry = targets
                    .entry(execution.target_json.clone())
                    .or_insert_with(|| base(execution.target_json));
                if !entry.retained_refs.contains(&execution.reference) {
                    entry.retained_refs.push(execution.reference);
                }
                if let Some(context) = execution.context {
                    if !entry.retained_contexts.contains(&context) {
                        entry.retained_contexts.push(context);
                    }
                }
            }
            dependents.extend(targets.into_values());
        }
        Ok(dependents)
    }

    pub(crate) async fn auth_live_dependents(
        &self,
        key: &str,
    ) -> StoreResult<Vec<AuthDependentTarget>> {
        let tx = self.begin().await?;
        let result = self.auth_live_dependents_tx(&tx, key).await?;
        tx.commit().await?;
        Ok(result)
    }
}

/// `MAX(h.revision)` over the outer fleet's revisions at or below its head.
fn latest_at_or_below_head() -> sea_orm::sea_query::SelectStatement {
    let history = Alias::new("h");
    Query::select()
        .expr(Expr::col((history.clone(), fleet_revisions::Column::Revision)).max())
        .from_as(fleet_revisions::Entity, history.clone())
        .and_where(
            Expr::col((history.clone(), fleet_revisions::Column::FleetKey))
                .equals((fleets::Entity, fleets::Column::Key)),
        )
        .and_where(
            Expr::col((history, fleet_revisions::Column::Revision))
                .lte(Expr::col((fleets::Entity, fleets::Column::DesiredRevision))),
        )
        .to_owned()
}
