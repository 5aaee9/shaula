//! A local process fence cannot prove that a remote Create was fully recorded.
//! Only a retained Create identity anchors that claim: the successful Create
//! result, or a failed Create whose bound state records the template's whole
//! managed resource shape (spec 0010 §7).
use super::{row::Row, sql, unavailable, worker::WorkerRow, SqliteStateBackend};
use sea_orm::{ConnectionTrait, DatabaseTransaction};
use shaula_core::state_backend::{StateDocument, StateError, StateResult};
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

/// `anchorable` means no writer of this state can remain, so a failed Create
/// whose state records the whole shape may be anchored here and now.
pub(super) async fn verified(
    tx: &DatabaseTransaction,
    id: &str,
    state: &StateDocument,
    anchorable: bool,
) -> StateResult<bool> {
    let row = tx
        .query_one(sql(
            "SELECT shaula_result_json, failed_create_identity_json FROM runner_generations WHERE id = ?",
            vec![id.into()],
        ))
        .await
        .map_err(unavailable)?
        .ok_or(StateError::Unavailable)?;
    let result = row
        .try_get::<Option<String>>("", "shaula_result_json")
        .map_err(unavailable)?;
    let failed = row
        .try_get::<Option<String>>("", "failed_create_identity_json")
        .map_err(unavailable)?;
    if matches(result.as_deref(), state) || matches(failed.as_deref(), state) {
        return Ok(true);
    }
    Ok(anchorable && result.is_none() && failed.is_none() && anchor(tx, id, state).await?)
}

/// Records the failed-Create anchor when the state records the whole shape
/// of the Generation's admitted manifest; `false` leaves it unanchored.
async fn anchor(tx: &DatabaseTransaction, id: &str, state: &StateDocument) -> StateResult<bool> {
    let manifest = tx
        .query_one(sql(
            "SELECT t.manifest_json FROM runner_generations g JOIN template_profile_revisions t ON t.profile_key = g.template_profile_key AND t.revision = g.template_revision AND t.artifact_digest = g.template_artifact_digest WHERE g.id = ?",
            vec![id.into()],
        ))
        .await
        .map_err(unavailable)?
        .and_then(|row| row.try_get::<Option<String>>("", "manifest_json").ok().flatten())
        .and_then(|text| serde_yaml::from_str::<ProfileManifest>(&text).ok());
    let Some(manifest) = manifest else {
        return Ok(false);
    };
    if !state.records_whole_shape(&manifest.managed_resource_shape) {
        return Ok(false);
    }
    let identity = serde_json::json!({
        "state_lineage": state.lineage(),
        "state_serial": state.serial(),
    })
    .to_string();
    tx.execute(sql(
        "UPDATE runner_generations SET failed_create_identity_json = ? WHERE id = ? AND shaula_result_json IS NULL AND failed_create_identity_json IS NULL",
        vec![identity.into(), id.into()],
    ))
    .await
    .map_err(unavailable)?;
    Ok(true)
}

impl SqliteStateBackend {
    /// Anchors a failed Create. No writer of the state may remain: the apply
    /// ended and released its lock, or the Worker was confirmed fenced (its
    /// orphan lock is then cleared by recovery). The state must still be
    /// authorized and started; otherwise the Generation keeps the unanchored
    /// path and is quarantined at Destroy.
    pub async fn anchor_failed_create(&self, id: &str) -> StateResult<bool> {
        let tx = self.writer(id).await?;
        let state = Row::load(&tx, id).await?;
        let fenced = WorkerRow::read(&tx, id)
            .await?
            .is_some_and(|worker| worker.phase == "fenced");
        if state.revoked
            || state.sealed
            || !state.create_started
            || (!fenced && state.lock(&tx).await?.is_some())
        {
            return Ok(false);
        }
        let Some(snapshot) = state.snapshot(&tx).await? else {
            return Ok(false);
        };
        let anchored = verified(&tx, id, &snapshot.document, true).await?;
        tx.commit().await.map_err(unavailable)?;
        Ok(anchored)
    }
}
