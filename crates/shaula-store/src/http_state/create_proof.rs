//! A local process fence cannot prove that a remote Create was fully recorded.
//! Only a retained Create identity anchors that claim: the successful Create
//! result, or a failed Create whose bound state records the template's whole
//! managed resource shape (spec 0010 §7).
use super::{row::Row, unavailable, worker::WorkerRow, SqliteStateBackend};
use crate::entities::lifecycle::runner_generations as generations;
use crate::entities::template::template_profile_revisions as revisions;
use crate::query_ext::{same, JoinOn};
use sea_orm::{
    sea_query::Expr, ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, QuerySelect,
};
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
    Ok(anchorable && result.is_none() && failed.is_none() && anchor(tx, id, state).await?)
}

/// Records the failed-Create anchor when the state records the whole shape
/// of the Generation's admitted manifest; `false` leaves it unanchored.
async fn anchor(tx: &DatabaseTransaction, id: &str, state: &StateDocument) -> StateResult<bool> {
    let manifest = generations::Entity::find_by_id(id)
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
