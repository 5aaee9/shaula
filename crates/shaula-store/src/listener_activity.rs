//! GitHub runner activity from listener job messages (spec 0001 lifecycle):
//! an observed start makes the exact Generation Busy and an observed
//! completion retires it, in the same transaction as the message facts.
use sea_orm::{sea_query::Expr, ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter};
use shaula_core::ports::PollMessage;

use crate::entities::lifecycle::runner_generations as generations;
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
                ["WaitingOnline", "Idle"],
            )
        });
        let completed = message.job_completed.iter().map(|j| {
            (
                j.runner_id,
                j.runner_name.as_str(),
                "Retiring",
                ["Idle", "Busy"],
            )
        });
        for (runner_id, runner_name, next, from) in started.chain(completed) {
            if runner_id <= 0 {
                continue;
            }
            generations::Entity::update_many()
                .col_expr(generations::Column::State, Expr::value(next))
                .col_expr(
                    generations::Column::Subphase,
                    Expr::value(Option::<String>::None),
                )
                .col_expr(generations::Column::UpdatedAt, Expr::value(now))
                .filter(generations::Column::FleetKey.eq(fleet))
                .filter(generations::Column::GithubRunnerId.eq(runner_id))
                .filter(generations::Column::RunnerName.eq(runner_name))
                .filter(generations::Column::State.is_in(from))
                .exec(tx)
                .await?;
        }
        Ok(())
    }
}
