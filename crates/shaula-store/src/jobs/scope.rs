//! Scope snapshots survive Fleet retirement and authentication revision rotation.
use sea_orm::{
    sea_query::{Expr, OnConflict},
    ActiveValue::Set,
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QuerySelect,
};
use sha2::Digest;
use shaula_core::auth_context::ResolvedAuthContext;
use shaula_core::registry::SessionEffectContext;

use super::decode;
use crate::entities::{
    auth::fleet_auth_contexts,
    fleet::{fleet_revisions, fleets},
    jobs::workflow_generation_identity as identity,
    lifecycle::{fleet_sessions, scale_set_state},
};
use crate::query_ext::{same, JoinOn};
use crate::{Store, StoreResult};

pub(super) struct JobScope {
    pub key: String,
    pub incarnation: String,
    pub scale_set_id: i64,
}

fn scope_key(fleet: &str, incarnation: &str, id: i64, auth: &ResolvedAuthContext) -> String {
    // Authentication profile/revision and session epoch are deliberately absent.
    let identity = serde_json::json!([
        fleet,
        incarnation,
        id,
        auth.github_host.to_ascii_lowercase(),
        auth.target.config_url().to_ascii_lowercase(),
        auth.account_id,
        auth.organization_id,
        auth.repository_id
    ]);
    hex::encode(sha2::Sha256::digest(identity.to_string().as_bytes()))
}

pub(super) async fn observed_scope<C: ConnectionTrait>(
    db: &C,
    fleet: &str,
    context: &SessionEffectContext,
) -> StoreResult<Option<JobScope>> {
    let found = fleet_sessions::Entity::find()
        .select_only()
        .column(fleet_sessions::Column::ScaleSetId)
        .filter(fleet_sessions::Column::FleetKey.eq(fleet))
        .filter(fleet_sessions::Column::Epoch.eq(context.epoch))
        .into_tuple::<i64>()
        .one(db)
        .await?;
    let Some(scale_set_id) = found else {
        return Ok(None);
    };
    Ok(Some(JobScope {
        key: scope_key(
            fleet,
            &context.guard.incarnation,
            scale_set_id,
            &context.auth_context,
        ),
        incarnation: context.guard.incarnation.clone(),
        scale_set_id,
    }))
}

impl Store {
    pub(crate) async fn jobs_snapshot_generation<C: ConnectionTrait>(
        db: &C,
        generation: &str,
        fleet: &str,
        revision: i64,
    ) -> StoreResult<()> {
        let found = fleets::Entity::find()
            .select_only()
            .column(fleets::Column::Incarnation)
            .column(scale_set_state::Column::ScaleSetId)
            .column(fleet_auth_contexts::Column::ObservedContextJson)
            .inner_join_on(
                fleet_revisions::Entity,
                same(fleet_revisions::Column::FleetKey, fleets::Column::Key).and(same(
                    fleet_revisions::Column::Incarnation,
                    fleets::Column::Incarnation,
                )),
            )
            .inner_join_on(
                scale_set_state::Entity,
                same(scale_set_state::Column::FleetKey, fleets::Column::Key),
            )
            .inner_join_on(
                fleet_auth_contexts::Entity,
                same(fleet_auth_contexts::Column::FleetKey, fleets::Column::Key),
            )
            .filter(fleets::Column::Key.eq(fleet))
            .filter(fleet_revisions::Column::Revision.eq(revision))
            .filter(scale_set_state::Column::State.eq("Adopted"))
            .filter(scale_set_state::Column::ScaleSetId.is_not_null())
            .into_tuple::<(String, i64, Option<String>)>()
            .one(db)
            .await?;
        let Some((incarnation, id, Some(json))) = found else {
            return Ok(());
        };
        let auth: ResolvedAuthContext = decode(&json)?;
        identity::Entity::insert(identity::ActiveModel {
            generation_id: Set(generation.into()),
            scope_key: Set(scope_key(fleet, &incarnation, id, &auth)),
            fleet_incarnation: Set(incarnation),
            ..Default::default()
        })
        .on_conflict(OnConflict::new().do_nothing().to_owned())
        .exec_without_returning(db)
        .await?;
        Ok(())
    }

    pub(crate) async fn jobs_register_runner_on<C: ConnectionTrait>(
        db: &C,
        generation: &str,
        runner: i64,
    ) -> StoreResult<()> {
        identity::Entity::update_many()
            .col_expr(identity::Column::GithubRunnerId, Expr::value(runner))
            .filter(identity::Column::GenerationId.eq(generation))
            .filter(
                identity::Column::GithubRunnerId
                    .is_null()
                    .or(identity::Column::GithubRunnerId.eq(runner)),
            )
            .exec(db)
            .await?;
        let found = identity::Entity::find_by_id(generation)
            .select_only()
            .column(identity::Column::ScopeKey)
            .into_tuple::<String>()
            .one(db)
            .await?;
        if let Some(scope) = found {
            super::association::refresh_runner_jobs(db, &scope, runner).await?;
        }
        Ok(())
    }
}
