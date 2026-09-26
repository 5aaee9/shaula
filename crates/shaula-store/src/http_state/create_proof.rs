//! A local process fence cannot prove that a remote Create was fully recorded.
//! Only the daemon's retained successful Create identity anchors that claim.
use super::{sql, unavailable};
use sea_orm::{ConnectionTrait, DatabaseTransaction};
use shaula_core::state_backend::{StateDocument, StateError, StateResult};

pub(super) fn matches(result: Option<&str>, state: &StateDocument) -> bool {
    #[derive(serde::Deserialize)]
    struct Identity {
        state_lineage: String,
        state_serial: u64,
    }
    result
        .and_then(|json| serde_json::from_str::<Identity>(json).ok())
        .is_some_and(|original| {
            original.state_lineage == state.lineage()
                && u64::try_from(state.serial()).is_ok_and(|serial| serial >= original.state_serial)
        })
}

pub(super) async fn verified(
    tx: &DatabaseTransaction,
    id: &str,
    state: &StateDocument,
) -> StateResult<bool> {
    let row = tx
        .query_one(sql(
            "SELECT shaula_result_json FROM runner_generations WHERE id = ?",
            vec![id.into()],
        ))
        .await
        .map_err(unavailable)?
        .ok_or(StateError::Unavailable)?;
    let result = row
        .try_get::<Option<String>>("", "shaula_result_json")
        .map_err(unavailable)?;
    Ok(matches(result.as_deref(), state))
}
