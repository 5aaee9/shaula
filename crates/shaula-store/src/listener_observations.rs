//! Message-scoped observations and acquisition intents in the ingest transaction.
use sea_orm::{ConnectionTrait, DatabaseTransaction, DbBackend, Statement};
use shaula_core::ports::PollMessage;

use crate::{Store, StoreError, StoreResult};

impl Store {
    pub(crate) async fn listener_observations_tx(
        &self,
        tx: &DatabaseTransaction,
        fleet: &str,
        epoch: i64,
        message: &PollMessage,
        now: i64,
    ) -> StoreResult<()> {
        for job in &message.job_available {
            if job.runner_request_id <= 0 {
                return Err(StoreError::Corrupt("invalid runner request ID".into()));
            }
            tx.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
                "INSERT OR IGNORE INTO listener_acquisitions(fleet_key,epoch,message_id,runner_request_id,state,updated_at)
                 VALUES(?,?,?,?,'Pending',?)",
                [fleet.into(),epoch.into(),message.message_id.into(),job.runner_request_id.into(),now.into()])).await?;
        }
        let observations = message
            .job_assigned
            .iter()
            .map(|j| ("Assigned", j.runner_request_id, j.job_id.as_str(), None))
            .chain(message.job_started.iter().map(|j| {
                (
                    "Started",
                    j.runner_request_id,
                    j.job_id.as_str(),
                    Some(j.runner_name.as_str()),
                )
            }))
            .chain(message.job_completed.iter().map(|j| {
                (
                    "Completed",
                    j.runner_request_id,
                    j.job_id.as_str(),
                    Some(j.runner_name.as_str()),
                )
            }));
        for (kind, request_id, job_id, runner_name) in observations {
            if request_id < 0 {
                return Err(StoreError::Corrupt(
                    "invalid observed runner request ID".into(),
                ));
            }
            if request_id == 0 {
                // Direct assignments can lack a request identity. The complete
                // approved fact is retained by jobs_ingest_tx in this same
                // transaction; this legacy request-keyed index cannot hold it.
                continue;
            }
            tx.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
                "INSERT OR IGNORE INTO listener_job_observations(fleet_key,epoch,message_id,observation_kind,
                 runner_request_id,job_id,runner_name,observed_at) VALUES(?,?,?,?,?,?,?,?)",
                [fleet.into(),epoch.into(),message.message_id.into(),kind.into(),request_id.into(),job_id.into(),runner_name.into(),now.into()])).await?;
        }
        Ok(())
    }
}
