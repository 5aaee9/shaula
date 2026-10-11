//! `TemplatePoolStore` port for shared TemplatePool resources (spec 0037).

use super::mapping::{pool_failure_policy, pool_member_row};
use super::{core_err, SqliteControlPlane};
use async_trait::async_trait;
use shaula_core::error::CoreResult;
use shaula_core::registry::{ChangeView, MutationError, MutationFacts, TemplatePoolStore};
use shaula_core::template_pool::TemplatePoolRevision;

impl SqliteControlPlane {
    pub(crate) async fn template_pool_revision_row(
        &self,
        key: &str,
    ) -> CoreResult<Option<TemplatePoolRevision>> {
        let Some(revision) = self
            .store
            .template_pool_revision_latest(key)
            .await
            .map_err(core_err)?
        else {
            return Ok(None);
        };
        let members = self
            .store
            .template_pool_members(key, revision.revision)
            .await
            .map_err(core_err)?
            .into_iter()
            .map(pool_member_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(core_err)?;
        Ok(Some(TemplatePoolRevision {
            pool_key: revision.pool_key,
            revision: revision.revision,
            spec_json: revision.spec_json,
            failure_policy: pool_failure_policy(&revision.failure_policy).map_err(core_err)?,
            members,
            actor: revision.actor,
            created_at: revision.created_at,
        }))
    }
}

#[async_trait]
impl TemplatePoolStore for SqliteControlPlane {
    async fn template_pool_get(
        &self,
        key: &str,
    ) -> CoreResult<Option<shaula_core::template_pool::TemplatePoolHead>> {
        self.template_pool_get_read(key).await
    }
    async fn template_pool_list(&self) -> CoreResult<Vec<(String, i64, String)>> {
        self.template_pool_list_read().await
    }
    async fn template_pool_revision_latest(
        &self,
        key: &str,
    ) -> CoreResult<Option<shaula_core::template_pool::TemplatePoolRevision>> {
        self.template_pool_revision_row(key).await
    }

    async fn commit_template_pool_mutation(
        &self,
        facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>> {
        self.commit_template_pool_mutation_impl(facts).await
    }

    async fn commit_template_pool_noop(
        &self,
        key: &str,
        incarnation: &str,
        revision: i64,
        actor: &shaula_core::registry::Actor,
        idempotency: Option<shaula_core::registry::IdempotencyInsert>,
        now: i64,
    ) -> CoreResult<Result<(), MutationError>> {
        self.commit_template_pool_noop_impl(key, incarnation, revision, actor, idempotency, now)
            .await
    }

    async fn commit_template_pool_delete(
        &self,
        facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>> {
        self.commit_template_pool_delete_impl(facts).await
    }

    async fn template_pool_change_get(&self, change_id: &str) -> CoreResult<Option<ChangeView>> {
        self.template_pool_change_get_read(change_id).await
    }
}
