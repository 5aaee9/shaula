//! Coverage includes desired admission and every still-live execution target.
use crate::entities::fleet::{fleet_revisions, fleets};
use crate::store::{Store, StoreResult};
use sea_orm::{ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, QueryOrder};
use shaula_core::registry::AuthDependentTarget;

impl Store {
    pub(crate) async fn auth_live_dependents_tx(
        &self,
        tx: &DatabaseTransaction,
        key: &str,
    ) -> StoreResult<Vec<AuthDependentTarget>> {
        let mut dependents = Vec::new();
        for fleet in fleets::Entity::find().all(tx).await? {
            let Some(latest) = fleet_revisions::Entity::find()
                .filter(fleet_revisions::Column::FleetKey.eq(&fleet.key))
                .filter(fleet_revisions::Column::Revision.lte(fleet.desired_revision))
                .order_by_desc(fleet_revisions::Column::Revision)
                .one(tx)
                .await?
            else {
                continue;
            };
            // Forgejo DELETE advances the head without a synthetic spec revision.
            // Keep its scope/credential dependency until cleanup is complete.
            if latest.revision != fleet.desired_revision
                && !serde_json::from_str::<serde_json::Value>(&latest.spec_json)
                    .is_ok_and(|spec| spec["kind"] == "forgejo")
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
