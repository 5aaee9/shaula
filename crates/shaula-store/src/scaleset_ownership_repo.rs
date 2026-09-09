//! Acquisition intent and scale set ownership persistence, split to
//! keep files under 400 lines (AGENTS.md).

use sea_orm::ActiveValue::Set;
use sea_orm::EntityTrait;

use crate::entities::lifecycle::scale_set_state;
use crate::store::{Store, StoreResult};

impl Store {
    // ---- Scale set ownership ----

    pub(crate) async fn scale_set_upsert(
        &self,
        row: shaula_core::registry::ScaleSetRow,
    ) -> StoreResult<()> {
        let fleet_key = &row.fleet_key;
        let scale_set_id = row.scale_set_id;
        let name = row.name.as_str();
        let runner_group = row.runner_group.as_str();
        let fingerprint = row.fingerprint.as_str();
        let state = row.state.as_str();
        let attempt_id = row.attempt_id.as_deref();
        let now = row.now;
        let tx = self.begin().await?;
        let existing = scale_set_state::Entity::find_by_id(fleet_key.to_string())
            .one(&tx)
            .await?;
        // Ownership is a store-managed historical fact, not a caller assertion.
        // A lookup candidate can have an ID without ever having been adopted.
        let owned_scale_set_id = owned_id(&row, existing.as_ref());
        match existing {
            Some(current) => {
                let mut updated: scale_set_state::ActiveModel = current.into();
                if scale_set_id.is_some() {
                    updated.scale_set_id = Set(scale_set_id);
                }
                updated.owned_scale_set_id = Set(owned_scale_set_id);
                updated.name = Set(name.to_string());
                updated.runner_group = Set(runner_group.to_string());
                updated.fingerprint = Set(fingerprint.to_string());
                updated.state = Set(state.to_string());
                if attempt_id.is_some() {
                    updated.attempt_id = Set(attempt_id.map(str::to_string));
                }
                updated.updated_at = Set(now);
                scale_set_state::Entity::update(updated).exec(&tx).await?;
            }
            None => {
                let row = scale_set_state::ActiveModel {
                    fleet_key: Set(fleet_key.to_string()),
                    scale_set_id: Set(scale_set_id),
                    owned_scale_set_id: Set(owned_scale_set_id),
                    name: Set(name.to_string()),
                    runner_group: Set(runner_group.to_string()),
                    fingerprint: Set(fingerprint.to_string()),
                    state: Set(state.to_string()),
                    attempt_id: Set(attempt_id.map(str::to_string)),
                    created_at: Set(now),
                    updated_at: Set(now),
                };
                scale_set_state::Entity::insert(row).exec(&tx).await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn scale_set_get(
        &self,
        fleet_key: &str,
    ) -> StoreResult<Option<scale_set_state::Model>> {
        Ok(scale_set_state::Entity::find_by_id(fleet_key.to_string())
            .one(self.connection())
            .await?)
    }
}

fn owned_id(
    next: &shaula_core::registry::ScaleSetRow,
    previous: Option<&scale_set_state::Model>,
) -> Option<i64> {
    if next.state == "Adopted" {
        return next.scale_set_id.filter(|id| *id > 0);
    }
    if !matches!(
        next.state.as_str(),
        "AccessBlocked" | "UnknownRemoteRunner" | "LabelsUpdating" | "LabelsUpdateUncertain"
    ) {
        // Reopen/create/missing states clear proof even when the candidate ID
        // remains for diagnostics under the existing upsert contract.
        return None;
    }
    let previous = previous?;
    let effective_id = next.scale_set_id.or(previous.scale_set_id);
    (previous.scale_set_id == effective_id
        && previous.owned_scale_set_id == effective_id
        && previous.fingerprint == next.fingerprint
        && previous.name == next.name
        && previous.runner_group == next.runner_group)
        .then_some(previous.owned_scale_set_id)
        .flatten()
}
