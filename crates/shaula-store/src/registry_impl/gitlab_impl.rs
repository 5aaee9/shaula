//! `GitlabPoolStore` port implementation.

use async_trait::async_trait;
use shaula_core::error::CoreResult;
use shaula_core::registry::{GitlabAnchorRow, GitlabAnchorState, GitlabJobAssociation};

use super::core_err;
use super::SqliteControlPlane;

#[async_trait]
impl shaula_core::registry::GitlabPoolStore for SqliteControlPlane {
    async fn generation_set_gitlab_runner(
        &self,
        id: &str,
        runner_id: i64,
        now: i64,
    ) -> CoreResult<()> {
        self.store
            .generation_set_gitlab_runner(id, runner_id, now)
            .await
            .map_err(core_err)
    }

    async fn generation_gitlab_runner(&self, id: &str) -> CoreResult<Option<i64>> {
        self.store
            .generation_gitlab_runner(id)
            .await
            .map_err(core_err)
    }

    async fn generation_set_gitlab_job(
        &self,
        id: &str,
        association: GitlabJobAssociation,
    ) -> CoreResult<()> {
        self.store
            .generation_set_gitlab_job(id, association)
            .await
            .map_err(core_err)
    }

    async fn generation_gitlab_job(&self, id: &str) -> CoreResult<Option<GitlabJobAssociation>> {
        self.store.generation_gitlab_job(id).await.map_err(core_err)
    }

    async fn gitlab_anchors(&self, fleet_key: &str) -> CoreResult<Vec<GitlabAnchorRow>> {
        self.store.gitlab_anchors(fleet_key).await.map_err(core_err)
    }

    async fn gitlab_anchor_begin(
        &self,
        fleet_key: &str,
        fleet_revision: i64,
        anchor_name: &str,
        now: i64,
    ) -> CoreResult<()> {
        self.store
            .gitlab_anchor_begin(fleet_key, fleet_revision, anchor_name, now)
            .await
            .map_err(core_err)
    }

    async fn gitlab_anchor_set(
        &self,
        fleet_key: &str,
        fleet_revision: i64,
        runner_id: Option<i64>,
        state: GitlabAnchorState,
        now: i64,
    ) -> CoreResult<()> {
        self.store
            .gitlab_anchor_set(fleet_key, fleet_revision, runner_id, state, now)
            .await
            .map_err(core_err)
    }

    async fn gitlab_anchor_remove(&self, fleet_key: &str, fleet_revision: i64) -> CoreResult<()> {
        self.store
            .gitlab_anchor_remove(fleet_key, fleet_revision)
            .await
            .map_err(core_err)
    }
}
