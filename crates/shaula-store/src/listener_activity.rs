//! GitHub runner activity from listener job messages (spec 0001 lifecycle):
//! an observed start makes the exact Generation Busy and an observed
//! completion retires it, in the same transaction as the message facts.
use sea_orm::{ConnectionTrait, DatabaseTransaction, DbBackend, Statement};
use shaula_core::ports::PollMessage;

use crate::{Store, StoreResult};

impl Store {
    /// Matches fleet, recorded GitHub runner ID and runner name; only legal
    /// lifecycle edges are taken. A started job proves the runner is online,
    /// so WaitingOnline passes through Idle. Unmatched runners change nothing.
    pub(crate) async fn listener_activity_tx(
        &self,
        tx: &DatabaseTransaction,
        fleet: &str,
        message: &PollMessage,
        now: i64,
    ) -> StoreResult<()> {
        let started = message.job_started.iter().map(|j| {
            (
                j.runner_id,
                j.runner_name.as_str(),
                "Busy",
                "'WaitingOnline','Idle'",
            )
        });
        let completed = message.job_completed.iter().map(|j| {
            (
                j.runner_id,
                j.runner_name.as_str(),
                "Retiring",
                "'Idle','Busy'",
            )
        });
        for (runner_id, runner_name, next, from) in started.chain(completed) {
            if runner_id <= 0 {
                continue;
            }
            tx.execute(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                format!(
                    "UPDATE runner_generations SET state=?, subphase=NULL, updated_at=?
                     WHERE fleet_key=? AND github_runner_id=? AND runner_name=? AND state IN ({from})"
                ),
                [
                    next.into(),
                    now.into(),
                    fleet.into(),
                    runner_id.into(),
                    runner_name.into(),
                ],
            ))
            .await?;
        }
        Ok(())
    }
}
