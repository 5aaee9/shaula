//! Forgejo identities live outside the GitHub runner namespace.

use crate::entities::fleet::fleet_revisions;
use sea_orm::{ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};

use crate::entities::lifecycle::{forgejo_runner_identities, runner_generations};
use crate::store::{Store, StoreError, StoreResult};

impl Store {
    pub(crate) async fn generation_set_forgejo_runner(
        &self,
        id: &str,
        runner_id: i64,
        runner_uuid: &str,
        now: i64,
    ) -> StoreResult<()> {
        if runner_id <= 0
            || runner_uuid.trim().is_empty()
            || runner_uuid.len() > 128
            || runner_uuid.chars().any(char::is_control)
        {
            return Err(StoreError::Corrupt(
                "invalid Forgejo runner identity".into(),
            ));
        }
        let tx = self.begin().await?;
        let generation = runner_generations::Entity::find_by_id(id.to_string())
            .one(&tx)
            .await?
            .ok_or_else(|| StoreError::Corrupt(format!("generation {id} missing")))?;
        let forgejo = fleet_revisions::Entity::find()
            .filter(fleet_revisions::Column::FleetKey.eq(&generation.fleet_key))
            .filter(fleet_revisions::Column::Revision.eq(generation.fleet_revision))
            .one(&tx)
            .await?
            .and_then(|row| {
                serde_json::from_str::<shaula_core::fleet::FleetSpec>(&row.spec_json).ok()
            })
            .is_some_and(|spec| spec.kind == shaula_core::fleet::FleetProviderKind::Forgejo);
        if !forgejo {
            return Err(StoreError::Conflict {
                resource: "generation is not a Forgejo generation".into(),
            });
        }
        if generation.github_runner_id.is_some() {
            return Err(StoreError::Conflict {
                resource: "generation already has a GitHub runner identity".into(),
            });
        }
        let existing = forgejo_runner_identities::Entity::find_by_id(id.to_string())
            .one(&tx)
            .await?;
        if let Some(existing) = existing {
            if existing.runner_id != runner_id || existing.runner_uuid != runner_uuid {
                return Err(StoreError::Conflict {
                    resource: "generation Forgejo runner identity changed".into(),
                });
            }
        } else {
            forgejo_runner_identities::Entity::insert(forgejo_runner_identities::ActiveModel {
                generation_id: Set(id.to_string()),
                runner_id: Set(runner_id),
                runner_uuid: Set(runner_uuid.to_string()),
                created_at: Set(now),
                updated_at: Set(now),
            })
            .exec(&tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn generation_forgejo_runner(
        &self,
        id: &str,
    ) -> StoreResult<Option<(i64, String)>> {
        Ok(
            forgejo_runner_identities::Entity::find_by_id(id.to_string())
                .one(self.connection())
                .await?
                .map(|row| (row.runner_id, row.runner_uuid)),
        )
    }
}
