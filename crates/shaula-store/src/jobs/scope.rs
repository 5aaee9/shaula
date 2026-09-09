//! Scope snapshots survive Fleet retirement and authentication revision rotation.
use sea_orm::ConnectionTrait;
use sha2::Digest;
use shaula_core::auth_context::ResolvedAuthContext;
use shaula_core::registry::SessionEffectContext;

use super::{decode, execute, rows};
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
    let found = rows(
        db,
        "SELECT scale_set_id FROM fleet_sessions WHERE fleet_key=? AND epoch=?",
        vec![fleet.into(), context.epoch.into()],
    )
    .await?;
    let Some(row) = found.first() else {
        return Ok(None);
    };
    let scale_set_id: i64 = row.try_get("", "scale_set_id")?;
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
        let found = rows(db,
            "SELECT f.incarnation,s.scale_set_id,a.observed_context_json
             FROM fleets f JOIN fleet_revisions r ON r.fleet_key=f.key AND r.incarnation=f.incarnation
             JOIN scale_set_state s ON s.fleet_key=f.key
             JOIN fleet_auth_contexts a ON a.fleet_key=f.key
             WHERE f.key=? AND r.revision=? AND s.state='Adopted' AND s.scale_set_id IS NOT NULL",
            vec![fleet.into(), revision.into()]).await?;
        let Some(row) = found.first() else {
            return Ok(());
        };
        let Some(json) = row.try_get::<Option<String>>("", "observed_context_json")? else {
            return Ok(());
        };
        let auth: ResolvedAuthContext = decode(&json)?;
        let incarnation: String = row.try_get("", "incarnation")?;
        let id: i64 = row.try_get("", "scale_set_id")?;
        execute(db, "INSERT OR IGNORE INTO workflow_generation_identity(generation_id,scope_key,fleet_incarnation)
            VALUES(?,?,?)", vec![generation.into(), scope_key(fleet, &incarnation, id, &auth).into(), incarnation.into()]).await
    }

    pub(crate) async fn jobs_register_runner_on<C: ConnectionTrait>(
        db: &C,
        generation: &str,
        runner: i64,
    ) -> StoreResult<()> {
        execute(
            db,
            "UPDATE workflow_generation_identity SET github_runner_id=?
            WHERE generation_id=? AND (github_runner_id IS NULL OR github_runner_id=?)",
            vec![runner.into(), generation.into(), runner.into()],
        )
        .await?;
        let found = rows(
            db,
            "SELECT scope_key FROM workflow_generation_identity WHERE generation_id=?",
            vec![generation.into()],
        )
        .await?;
        if let Some(row) = found.first() {
            let scope: String = row.try_get("", "scope_key")?;
            super::association::refresh_runner_jobs(db, &scope, runner).await?;
        }
        Ok(())
    }
}
