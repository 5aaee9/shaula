//! GitLab Pool persistence (m0030). Identities and associations are
//! write-once per Generation: a changed value is a conflict, never an update.

use sea_orm::{ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use shaula_core::registry::{GitlabAnchorRow, GitlabAnchorState, GitlabJobAssociation};

use crate::entities::fleet::fleet_revisions;
use crate::entities::gitlab::{gitlab_anchors, gitlab_job_associations, gitlab_runner_identities};
use crate::entities::lifecycle::runner_generations;
use crate::store::{Store, StoreError, StoreResult};

fn spec_is_gitlab(json: &str) -> bool {
    serde_json::from_str::<shaula_core::fleet::FleetSpec>(json)
        .is_ok_and(|spec| spec.kind == shaula_core::fleet::FleetProviderKind::Gitlab)
}

impl Store {
    pub(crate) async fn generation_set_gitlab_runner(
        &self,
        id: &str,
        runner_id: i64,
        now: i64,
    ) -> StoreResult<()> {
        if runner_id <= 0 {
            return Err(StoreError::Corrupt("invalid GitLab runner identity".into()));
        }
        let tx = self.begin().await?;
        let generation = runner_generations::Entity::find_by_id(id.to_string())
            .one(&tx)
            .await?
            .ok_or_else(|| StoreError::Corrupt(format!("generation {id} missing")))?;
        let gitlab = fleet_revisions::Entity::find()
            .filter(fleet_revisions::Column::FleetKey.eq(&generation.fleet_key))
            .filter(fleet_revisions::Column::Revision.eq(generation.fleet_revision))
            .one(&tx)
            .await?
            .is_some_and(|row| spec_is_gitlab(&row.spec_json));
        if !gitlab || generation.github_runner_id.is_some() {
            return Err(StoreError::Conflict {
                resource: "generation is not a GitLab generation".into(),
            });
        }
        match gitlab_runner_identities::Entity::find_by_id(id.to_string())
            .one(&tx)
            .await?
        {
            Some(existing) if existing.runner_id != runner_id => {
                return Err(StoreError::Conflict {
                    resource: "generation GitLab runner identity changed".into(),
                });
            }
            Some(_) => {}
            None => {
                gitlab_runner_identities::Entity::insert(gitlab_runner_identities::ActiveModel {
                    generation_id: Set(id.to_string()),
                    runner_id: Set(runner_id),
                    created_at: Set(now),
                    updated_at: Set(now),
                })
                .exec(&tx)
                .await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn generation_gitlab_runner(&self, id: &str) -> StoreResult<Option<i64>> {
        Ok(gitlab_runner_identities::Entity::find_by_id(id.to_string())
            .one(self.connection())
            .await?
            .map(|row| row.runner_id))
    }

    pub(crate) async fn generation_set_gitlab_job(
        &self,
        id: &str,
        association: GitlabJobAssociation,
    ) -> StoreResult<()> {
        let tx = self.begin().await?;
        match gitlab_job_associations::Entity::find_by_id(id.to_string())
            .one(&tx)
            .await?
        {
            Some(existing) if existing.job_id != association.job_id => {
                return Err(StoreError::Conflict {
                    resource: "generation GitLab job association changed".into(),
                });
            }
            Some(_) => {}
            None => {
                gitlab_job_associations::Entity::insert(gitlab_job_associations::ActiveModel {
                    generation_id: Set(id.to_string()),
                    job_id: Set(association.job_id),
                    project_id: Set(association.project_id),
                    pipeline_id: Set(association.pipeline_id),
                    observed_at: Set(association.observed_at),
                })
                .exec(&tx)
                .await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn generation_gitlab_job(
        &self,
        id: &str,
    ) -> StoreResult<Option<GitlabJobAssociation>> {
        Ok(gitlab_job_associations::Entity::find_by_id(id.to_string())
            .one(self.connection())
            .await?
            .map(|row| GitlabJobAssociation {
                job_id: row.job_id,
                project_id: row.project_id,
                pipeline_id: row.pipeline_id,
                observed_at: row.observed_at,
            }))
    }

    pub(crate) async fn gitlab_anchors(
        &self,
        fleet_key: &str,
    ) -> StoreResult<Vec<GitlabAnchorRow>> {
        gitlab_anchors::Entity::find()
            .filter(gitlab_anchors::Column::FleetKey.eq(fleet_key))
            .order_by_asc(gitlab_anchors::Column::FleetRevision)
            .all(self.connection())
            .await?
            .into_iter()
            .map(|row| {
                Ok(GitlabAnchorRow {
                    state: GitlabAnchorState::parse(&row.state)
                        .ok_or_else(|| StoreError::Corrupt("unknown GitLab anchor state".into()))?,
                    fleet_key: row.fleet_key,
                    fleet_revision: row.fleet_revision,
                    anchor_name: row.anchor_name,
                    runner_id: row.runner_id,
                })
            })
            .collect()
    }

    pub(crate) async fn gitlab_anchor_begin(
        &self,
        fleet_key: &str,
        fleet_revision: i64,
        anchor_name: &str,
        now: i64,
    ) -> StoreResult<()> {
        let tx = self.begin().await?;
        let existing = gitlab_anchors::Entity::find_by_id((fleet_key.to_string(), fleet_revision))
            .one(&tx)
            .await?;
        if existing.is_none() {
            gitlab_anchors::Entity::insert(gitlab_anchors::ActiveModel {
                fleet_key: Set(fleet_key.to_string()),
                fleet_revision: Set(fleet_revision),
                anchor_name: Set(anchor_name.to_string()),
                runner_id: Set(None),
                state: Set(GitlabAnchorState::Registering.as_str().into()),
                created_at: Set(now),
                updated_at: Set(now),
            })
            .exec(&tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn gitlab_anchor_set(
        &self,
        fleet_key: &str,
        fleet_revision: i64,
        runner_id: Option<i64>,
        state: GitlabAnchorState,
        now: i64,
    ) -> StoreResult<()> {
        let tx = self.begin().await?;
        let row = gitlab_anchors::Entity::find_by_id((fleet_key.to_string(), fleet_revision))
            .one(&tx)
            .await?
            .ok_or_else(|| StoreError::Corrupt("GitLab anchor missing".into()))?;
        if row.runner_id.is_some() && runner_id.is_some() && row.runner_id != runner_id {
            return Err(StoreError::Conflict {
                resource: "GitLab anchor identity changed".into(),
            });
        }
        let mut active: gitlab_anchors::ActiveModel = row.clone().into();
        active.runner_id = Set(runner_id.or(row.runner_id));
        active.state = Set(state.as_str().into());
        active.updated_at = Set(now);
        gitlab_anchors::Entity::update(active).exec(&tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn gitlab_anchor_remove(
        &self,
        fleet_key: &str,
        fleet_revision: i64,
    ) -> StoreResult<()> {
        gitlab_anchors::Entity::delete_by_id((fleet_key.to_string(), fleet_revision))
            .exec(self.connection())
            .await?;
        Ok(())
    }
}
