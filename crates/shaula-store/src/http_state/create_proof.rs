//! A local process fence cannot prove that a remote Create was fully recorded.
//! Only a retained Create identity anchors that claim: the successful Create
//! result, or a failed Create whose bound state records the template's whole
//! managed resource shape (spec 0010 §7).
use super::{row::Row, set_worker, unavailable, worker::WorkerRow, workers, SqliteStateBackend};
use crate::entities::lifecycle::runner_generations as generations;
use crate::entities::template::template_profile_revisions as revisions;
use crate::query_ext::{same, JoinOn};
use sea_orm::{
    sea_query::Expr, ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, QueryFilter,
    QuerySelect,
};
use shaula_core::state_backend::{
    AnchorOutcome, AnchorSkipReason, StateDocument, StateError, StateResult,
};
use shaula_core::template::ProfileManifest;

#[derive(serde::Deserialize)]
struct Identity {
    state_lineage: String,
    state_serial: u64,
}

pub(super) fn matches(result: Option<&str>, state: &StateDocument) -> bool {
    result
        .and_then(|json| serde_json::from_str::<Identity>(json).ok())
        .is_some_and(|original| {
            original.state_lineage == state.lineage()
                && u64::try_from(state.serial()).is_ok_and(|serial| serial >= original.state_serial)
        })
}

/// Whether the Generation already holds a retained Create identity (successful
/// `shaula_result_json` or anchored `failed_create_identity_json`) matching
/// `state`. Unanchored partial/empty states admitted for `reconcile` return
/// `false` until `anchor_reconciled` succeeds.
pub(super) async fn is_anchored(
    conn: &impl ConnectionTrait,
    id: &str,
    state: &StateDocument,
) -> StateResult<bool> {
    let (result, failed) = generations::Entity::find_by_id(id)
        .select_only()
        .columns([
            generations::Column::ShaulaResultJson,
            generations::Column::FailedCreateIdentityJson,
        ])
        .into_tuple::<(Option<String>, Option<String>)>()
        .one(conn)
        .await
        .map_err(unavailable)?
        .ok_or(StateError::Unavailable)?;
    Ok(matches(result.as_deref(), state) || matches(failed.as_deref(), state))
}

/// `anchorable` means no writer of this state can remain, so a failed Create
/// whose state records the whole shape may be anchored here and now.
pub(super) async fn verified(
    tx: &DatabaseTransaction,
    id: &str,
    state: &StateDocument,
    anchorable: bool,
) -> StateResult<bool> {
    let (result, failed) = generations::Entity::find_by_id(id)
        .select_only()
        .columns([
            generations::Column::ShaulaResultJson,
            generations::Column::FailedCreateIdentityJson,
        ])
        .into_tuple::<(Option<String>, Option<String>)>()
        .one(tx)
        .await
        .map_err(unavailable)?
        .ok_or(StateError::Unavailable)?;
    if matches(result.as_deref(), state) || matches(failed.as_deref(), state) {
        return Ok(true);
    }
    Ok(anchorable
        && result.is_none()
        && failed.is_none()
        && anchor(tx, id, state).await?.is_anchored())
}

async fn load_manifest(tx: &DatabaseTransaction, id: &str) -> StateResult<Option<ProfileManifest>> {
    Ok(generations::Entity::find_by_id(id)
        .select_only()
        .column(revisions::Column::ManifestJson)
        .inner_join_on(
            revisions::Entity,
            same(
                revisions::Column::ProfileKey,
                generations::Column::TemplateProfileKey,
            )
            .and(same(
                revisions::Column::Revision,
                generations::Column::TemplateRevision,
            ))
            .and(same(
                revisions::Column::ArtifactDigest,
                generations::Column::TemplateArtifactDigest,
            )),
        )
        .into_tuple::<Option<String>>()
        .one(tx)
        .await
        .map_err(unavailable)?
        .flatten()
        .and_then(|text| serde_yaml::from_str::<ProfileManifest>(&text).ok()))
}

/// Whether this Generation's admitted manifest declares an optional read-only
/// failed-Create discovery module (ARD-0044).
pub(super) async fn manifest_supports_reconcile(
    tx: &DatabaseTransaction,
    id: &str,
) -> StateResult<bool> {
    Ok(load_manifest(tx, id)
        .await?
        .is_some_and(|manifest| manifest.reconcile.is_some()))
}

/// Records the failed-Create anchor when the state records the whole shape
/// of the Generation's admitted manifest; `Skipped` leaves it unanchored with
/// an explicit reason.
async fn anchor(
    tx: &DatabaseTransaction,
    id: &str,
    state: &StateDocument,
) -> StateResult<AnchorOutcome> {
    let Some(manifest) = load_manifest(tx, id).await? else {
        return Ok(AnchorOutcome::Skipped(
            AnchorSkipReason::ManifestUnavailable,
        ));
    };
    if !state.records_whole_shape(&manifest.managed_resource_shape) {
        return Ok(AnchorOutcome::Skipped(AnchorSkipReason::ShapeIncomplete));
    }
    write_failed_create_identity(tx, id, state.lineage(), state.serial()).await?;
    Ok(AnchorOutcome::Anchored)
}

async fn write_failed_create_identity(
    tx: &DatabaseTransaction,
    id: &str,
    lineage: &str,
    serial: i64,
) -> StateResult<()> {
    let identity = serde_json::json!({
        "state_lineage": lineage,
        "state_serial": serial,
    })
    .to_string();
    generations::Entity::update_many()
        .col_expr(
            generations::Column::FailedCreateIdentityJson,
            Expr::value(identity),
        )
        .filter(generations::Column::Id.eq(id))
        .filter(generations::Column::ShaulaResultJson.is_null())
        .filter(generations::Column::FailedCreateIdentityJson.is_null())
        .exec(tx)
        .await
        .map_err(unavailable)?;
    Ok(())
}

impl SqliteStateBackend {
    /// Anchors a failed Create. No writer of the state may remain: the apply
    /// ended and released its lock, or the Worker was confirmed fenced (its
    /// orphan lock is then cleared by recovery). Returns a structured skip
    /// reason so logs and diagnostics stay actionable (ARD-0044 Phase 1).
    pub async fn anchor_failed_create(&self, id: &str) -> StateResult<AnchorOutcome> {
        let tx = self.writer().await?;
        let state = Row::load(&tx, id).await?;
        if state.revoked || state.sealed {
            return Ok(AnchorOutcome::Skipped(AnchorSkipReason::RevokedOrSealed));
        }
        if !state.create_started {
            return Ok(AnchorOutcome::Skipped(AnchorSkipReason::NotStarted));
        }
        let fenced = WorkerRow::read(&tx, id)
            .await?
            .is_some_and(|worker| worker.phase == "fenced");
        if !fenced && state.lock(&tx).await?.is_some() {
            return Ok(AnchorOutcome::Skipped(AnchorSkipReason::LockHeld));
        }
        let Some(snapshot) = state.snapshot(&tx).await? else {
            return Ok(AnchorOutcome::Skipped(AnchorSkipReason::NoSnapshot));
        };
        let (result, failed) = generations::Entity::find_by_id(id)
            .select_only()
            .columns([
                generations::Column::ShaulaResultJson,
                generations::Column::FailedCreateIdentityJson,
            ])
            .into_tuple::<(Option<String>, Option<String>)>()
            .one(&tx)
            .await
            .map_err(unavailable)?
            .ok_or(StateError::Unavailable)?;
        if matches(result.as_deref(), &snapshot.document)
            || matches(failed.as_deref(), &snapshot.document)
        {
            tx.commit().await.map_err(unavailable)?;
            return Ok(AnchorOutcome::Anchored);
        }
        if result.is_some() || failed.is_some() {
            return Ok(AnchorOutcome::Skipped(AnchorSkipReason::ShapeIncomplete));
        }
        let outcome = anchor(&tx, id, &snapshot.document).await?;
        tx.commit().await.map_err(unavailable)?;
        Ok(outcome)
    }

    /// Anchors a failed Create after the template's read-only `discover`
    /// module (and any targeted `import`) proved every remote resource is
    /// accounted for in the current authoritative state (ARD-0044 Phase 2).
    /// When `empty` is true, also records `cleanup_revision` atomically so
    /// terminal sealing succeeds without a no-op Destroy apply.
    pub async fn anchor_reconciled(
        &self,
        id: &str,
        lineage: &str,
        serial: u64,
        empty: bool,
    ) -> StateResult<bool> {
        let Ok(expected_serial) = i64::try_from(serial) else {
            return Ok(false);
        };
        let tx = self.writer().await?;
        let state = Row::load(&tx, id).await?;
        if state.revoked
            || state.sealed
            || !state.create_started
            || state.lock(&tx).await?.is_some()
        {
            return Ok(false);
        }
        if let Some(worker) = WorkerRow::read(&tx, id).await? {
            if worker.handover.is_some()
                || worker.active_command.is_some()
                || !matches!(worker.phase.as_str(), "running" | "fenced" | "admitted")
            {
                return Ok(false);
            }
        }
        if !manifest_supports_reconcile(&tx, id).await? {
            return Ok(false);
        }
        let Some(snapshot) = state.snapshot(&tx).await? else {
            return Ok(false);
        };
        if snapshot.document.lineage() != lineage
            || snapshot.document.serial() != expected_serial
            || snapshot.document.managed_empty() != empty
        {
            return Ok(false);
        }
        write_failed_create_identity(&tx, id, lineage, expected_serial).await?;
        if empty {
            set_worker(
                &tx,
                id,
                [(
                    workers::Column::CleanupRevision,
                    Expr::value(state.revision),
                )],
            )
            .await?;
        }
        tx.commit().await.map_err(unavailable)?;
        Ok(true)
    }
}
