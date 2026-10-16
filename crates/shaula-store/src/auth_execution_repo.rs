//! Exact execution references and immutable context retention.
use crate::entities::auth::fleet_auth_context_history as history;
use crate::entities::fleet::fleet_revisions;
use crate::entities::lifecycle::{
    runner_generations as generations, runner_operations as operations,
};
use crate::query_ext::{same, JoinOn};
use crate::store::{Store, StoreError, StoreResult};
use sea_orm::{
    sea_query::OnConflict, ActiveValue::Set, ColumnTrait, DatabaseTransaction, EntityTrait,
    QueryFilter, QueryOrder, QuerySelect,
};
use shaula_core::auth_context::ResolvedAuthContext;

#[path = "auth_execution_deps.rs"]
mod deps;

impl Store {
    pub(crate) async fn auth_archive_context_tx(
        &self,
        tx: &DatabaseTransaction,
        fleet: &str,
        context: &ResolvedAuthContext,
    ) -> StoreResult<()> {
        let json =
            serde_json::to_string(context).map_err(|e| StoreError::Corrupt(e.to_string()))?;
        let previous = self
            .auth_execution_context_tx(tx, fleet, &context.profile_key, context.revision)
            .await?;
        if previous.as_ref().is_some_and(|old| old != context) {
            return Err(StoreError::Corrupt(
                "attempt to replace retained execution context".into(),
            ));
        }
        history::Entity::insert(history::ActiveModel {
            fleet_key: Set(fleet.to_owned()),
            profile_key: Set(context.profile_key.clone()),
            revision: Set(context.revision),
            context_json: Set(json),
        })
        .on_conflict(
            OnConflict::columns([
                history::Column::FleetKey,
                history::Column::ProfileKey,
                history::Column::Revision,
            ])
            .do_nothing()
            .to_owned(),
        )
        .exec_without_returning(tx)
        .await?;
        Ok(())
    }

    pub(crate) async fn auth_execution_context_tx(
        &self,
        tx: &DatabaseTransaction,
        fleet: &str,
        key: &str,
        revision: i64,
    ) -> StoreResult<Option<ResolvedAuthContext>> {
        let row = history::Entity::find_by_id((fleet.to_owned(), key.to_owned(), revision))
            .one(tx)
            .await?;
        row.map(|row| {
            let context: ResolvedAuthContext = serde_json::from_str(&row.context_json)
                .map_err(|e| StoreError::Corrupt(format!("retained auth context corrupt: {e}")))?;
            if context.profile_key != key
                || context.revision != revision
                || !context.has_complete_identity()
            {
                return Err(StoreError::Corrupt(
                    "retained auth context does not match its reference".into(),
                ));
            }
            Ok(context)
        })
        .transpose()
    }

    pub(crate) async fn auth_execution_context_get(
        &self,
        fleet: &str,
        key: &str,
        revision: i64,
    ) -> StoreResult<Option<ResolvedAuthContext>> {
        let tx = self.begin().await?;
        let row = self
            .auth_revision_get_tx(&tx, key, revision)
            .await?
            .ok_or_else(|| StoreError::Corrupt("execution auth revision missing".into()))?;
        if row.schema_version != 2 || row.kind != "github_app" {
            return Err(StoreError::PolicyDenied {
                reason: "UnsupportedAuthFormat",
            });
        }
        let result = self
            .auth_execution_context_tx(&tx, fleet, key, revision)
            .await?;
        tx.commit().await?;
        Ok(result)
    }

    pub(crate) async fn auth_generation_ref_tx(
        &self,
        tx: &DatabaseTransaction,
        generation: &str,
    ) -> StoreResult<Option<(String, i64)>> {
        let provenance = operations::Entity::find()
            .select_only()
            .column(operations::Column::ProvenanceJson)
            .filter(operations::Column::GenerationId.eq(generation))
            .filter(operations::Column::Kind.eq("JitStarting"))
            .order_by_asc(operations::Column::CreatedAt)
            .limit(1)
            .into_tuple::<Option<String>>()
            .one(tx)
            .await?
            .flatten();
        if let Some(json) = provenance {
            let value: serde_json::Value = serde_json::from_str(&json)
                .map_err(|e| StoreError::Corrupt(format!("JIT auth provenance corrupt: {e}")))?;
            if let Some(key) = value.pointer("/context/auth_profile_key") {
                let key = key
                    .as_str()
                    .ok_or_else(|| StoreError::Corrupt("JIT auth key invalid".into()))?;
                let revision = value
                    .pointer("/context/auth_revision")
                    .and_then(|v| v.as_i64())
                    .ok_or_else(|| StoreError::Corrupt("JIT auth revision missing".into()))?;
                return Ok(Some((key.to_string(), revision)));
            }
        }
        Ok(generations::Entity::find_by_id(generation)
            .select_only()
            .columns([
                fleet_revisions::Column::AuthDesiredProfileKey,
                fleet_revisions::Column::AuthDesiredRevision,
            ])
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
            .into_tuple::<(String, i64)>()
            .one(tx)
            .await?)
    }

    pub(crate) async fn auth_generation_ref(
        &self,
        generation: &str,
    ) -> StoreResult<Option<(String, i64)>> {
        let tx = self.begin().await?;
        let result = self.auth_generation_ref_tx(&tx, generation).await?;
        tx.commit().await?;
        Ok(result)
    }
}

pub(crate) fn target_from_spec(spec: &str) -> StoreResult<String> {
    let spec: serde_json::Value =
        serde_json::from_str(spec).map_err(|e| StoreError::Corrupt(e.to_string()))?;
    if spec["kind"] == "gitlab" {
        let target = shaula_core::gitlab::GitlabTarget {
            instance_url: serde_json::from_value(spec["gitlab"]["instance_url"].clone())
                .map_err(|e| StoreError::Corrupt(format!("GitLab instance corrupt: {e}")))?,
            scope: serde_json::from_value(spec["gitlab"]["scope"].clone())
                .map_err(|e| StoreError::Corrupt(format!("GitLab scope corrupt: {e}")))?,
        };
        return serde_json::to_string(&target).map_err(|e| StoreError::Corrupt(e.to_string()));
    }
    if spec["kind"] == "forgejo" {
        let target = shaula_core::forgejo::ForgejoTarget {
            instance_url: serde_json::from_value(spec["forgejo"]["instance_url"].clone())
                .map_err(|e| StoreError::Corrupt(format!("Forgejo instance corrupt: {e}")))?,
            scope: serde_json::from_value(spec["forgejo"]["scope"].clone())
                .map_err(|e| StoreError::Corrupt(format!("Forgejo scope corrupt: {e}")))?,
        };
        return serde_json::to_string(&target).map_err(|e| StoreError::Corrupt(e.to_string()));
    }
    let target: shaula_core::github::GitHubTarget =
        serde_json::from_value(spec["github"]["target"].clone())
            .map_err(|e| StoreError::Corrupt(format!("fleet target corrupt: {e}")))?;
    serde_json::to_string(&target).map_err(|e| StoreError::Corrupt(e.to_string()))
}
