//! Message-scoped observations and acquisition intents in the ingest transaction.
use sea_orm::{sea_query::OnConflict, ActiveValue::Set, DatabaseTransaction, EntityTrait};
use shaula_core::ports::PollMessage;

use crate::entities::listener::{
    listener_acquisitions as acquisitions, listener_job_observations as observations,
};
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
            acquisitions::Entity::insert(acquisitions::ActiveModel {
                fleet_key: Set(fleet.to_owned()),
                epoch: Set(epoch),
                message_id: Set(message.message_id),
                runner_request_id: Set(job.runner_request_id),
                state: Set("Pending".to_owned()),
                updated_at: Set(now),
            })
            .on_conflict(
                OnConflict::columns([
                    acquisitions::Column::FleetKey,
                    acquisitions::Column::Epoch,
                    acquisitions::Column::RunnerRequestId,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(tx)
            .await?;
        }
        let observed = message
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
        for (kind, request_id, job_id, runner_name) in observed {
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
            observations::Entity::insert(observations::ActiveModel {
                fleet_key: Set(fleet.to_owned()),
                epoch: Set(epoch),
                message_id: Set(message.message_id),
                observation_kind: Set(kind.to_owned()),
                runner_request_id: Set(request_id),
                job_id: Set(job_id.to_owned()),
                runner_name: Set(runner_name.map(str::to_owned)),
                observed_at: Set(now),
            })
            .on_conflict(
                OnConflict::columns([
                    observations::Column::FleetKey,
                    observations::Column::Epoch,
                    observations::Column::MessageId,
                    observations::Column::ObservationKind,
                    observations::Column::RunnerRequestId,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(tx)
            .await?;
        }
        Ok(())
    }
}
