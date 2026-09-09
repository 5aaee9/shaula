//! Retained message facts are committed in the listener's existing transaction.
use std::collections::BTreeSet;

use sea_orm::DatabaseTransaction;
use shaula_core::jobs::{
    AssociationStatus, JobMetadata, JobObservation, JobSummary, ObservationKind, ObservedStatus,
};
use shaula_core::ports::PollMessage;
use shaula_core::registry::SessionEffectContext;

use super::{association, encode, execute, rows, scope};
use crate::{Store, StoreError, StoreResult};

struct Fact<'a> {
    kind: ObservationKind,
    request: i64,
    job: &'a str,
    runner: Option<i64>,
    name: Option<&'a str>,
    metadata: &'a JobMetadata,
    result: Option<&'a str>,
}

fn facts(message: &PollMessage) -> Vec<Fact<'_>> {
    let mut facts = Vec::new();
    for (kind, jobs) in [
        (ObservationKind::Available, &message.job_available),
        (ObservationKind::Assigned, &message.job_assigned),
    ] {
        facts.extend(jobs.iter().map(|j| Fact {
            kind,
            request: j.runner_request_id,
            job: &j.job_id,
            runner: None,
            name: None,
            metadata: &j.metadata,
            result: None,
        }));
    }
    facts.extend(message.job_started.iter().map(|j| Fact {
        kind: ObservationKind::Started,
        request: j.runner_request_id,
        job: &j.job_id,
        runner: (j.runner_id > 0).then_some(j.runner_id),
        name: (!j.runner_name.is_empty()).then_some(j.runner_name.as_str()),
        metadata: &j.metadata,
        result: None,
    }));
    facts.extend(message.job_completed.iter().map(|j| Fact {
        kind: ObservationKind::Completed,
        request: j.runner_request_id,
        job: &j.job_id,
        runner: (j.runner_id > 0).then_some(j.runner_id),
        name: (!j.runner_name.is_empty()).then_some(j.runner_name.as_str()),
        metadata: &j.metadata,
        result: j.result.as_deref(),
    }));
    facts
}

impl Store {
    pub(crate) async fn jobs_ingest_tx(
        &self,
        tx: &DatabaseTransaction,
        fleet: &str,
        context: &SessionEffectContext,
        message: &PollMessage,
        now: i64,
    ) -> StoreResult<()> {
        let Some(scope) = scope::observed_scope(tx, fleet, context).await? else {
            return Err(StoreError::Corrupt(
                "job observation lacks session identity".into(),
            ));
        };
        let mut affected = BTreeSet::new();
        let mut runners = BTreeSet::new();
        let mut requests = BTreeSet::new();
        for fact in facts(message) {
            if fact.request < 0
                || (fact.request == 0 && fact.kind == ObservationKind::Available)
                || fact.job.len() > 4096
                || fact.name.is_some_and(|n| n.len() > 1024)
            {
                return Err(StoreError::Corrupt(
                    "invalid job observation identity".into(),
                ));
            }
            let job_id = if fact.job.trim().is_empty() {
                None
            } else {
                Some(ensure_job(tx, fleet, &scope, fact.job, now).await?)
            };
            affected.extend(job_id.iter().cloned());
            runners.extend(fact.runner);
            // Zero is a wire sentinel, never a shared request identity.
            if fact.request > 0 {
                requests.insert(fact.request);
            }
            let observation = JobObservation {
                id: uuid::Uuid::new_v4().to_string(),
                kind: fact.kind,
                runner_request_id: fact.request,
                runner_id: fact.runner,
                runner_name: fact.name.map(str::to_owned),
                metadata: fact.metadata.clone(),
                reported_result: fact.result.map(str::to_owned),
                epoch: context.epoch,
                message_id: message.message_id,
                observed_at: now,
                generation_id: None,
                association_status: AssociationStatus::Unverified,
            };
            execute(
                tx,
                "INSERT INTO workflow_job_observations(id,scope_key,job_record_id,protocol_job_id,
                runner_request_id,runner_id,data_json,observed_at) VALUES(?,?,?,?,?,?,?,?)",
                vec![
                    observation.id.clone().into(),
                    scope.key.clone().into(),
                    job_id.into(),
                    (if fact.job.trim().is_empty() {
                        ""
                    } else {
                        fact.job
                    })
                    .into(),
                    fact.request.into(),
                    fact.runner.into(),
                    encode(&observation)?.into(),
                    now.into(),
                ],
            )
            .await?;
        }
        for request in requests {
            // Missing jobId can be joined only when all facts for this scoped
            // request agree. A later conflict retracts this tentative promotion.
            let candidates = rows(tx, "SELECT DISTINCT job_record_id FROM workflow_job_observations
                WHERE scope_key=? AND runner_request_id=? AND protocol_job_id<>'' AND job_record_id IS NOT NULL",
                vec![scope.key.clone().into(), request.into()]).await?;
            let candidate = if candidates.len() == 1 {
                Some(candidates[0].try_get::<String>("", "job_record_id")?)
            } else {
                None
            };
            execute(
                tx,
                "UPDATE workflow_job_observations SET job_record_id=?
                WHERE scope_key=? AND runner_request_id=? AND protocol_job_id=''",
                vec![candidate.into(), scope.key.clone().into(), request.into()],
            )
            .await?;
            for row in candidates {
                affected.insert(row.try_get("", "job_record_id")?);
            }
        }
        for id in affected {
            association::refresh_job(tx, &id).await?;
        }
        for runner in runners {
            association::refresh_runner_jobs(tx, &scope.key, runner).await?;
        }
        Ok(())
    }
}

async fn ensure_job(
    tx: &DatabaseTransaction,
    fleet: &str,
    scope: &scope::JobScope,
    protocol_id: &str,
    now: i64,
) -> StoreResult<String> {
    let found = rows(
        tx,
        "SELECT id FROM workflow_jobs WHERE scope_key=? AND protocol_job_id=?",
        vec![scope.key.clone().into(), protocol_id.into()],
    )
    .await?;
    if let Some(row) = found.first() {
        return Ok(row.try_get("", "id")?);
    }
    let summary = JobSummary {
        id: uuid::Uuid::new_v4().to_string(),
        fleet_key: fleet.into(),
        fleet_incarnation: scope.incarnation.clone(),
        scale_set_id: scope.scale_set_id,
        protocol_job_id: protocol_id.into(),
        metadata: JobMetadata::default(),
        observed_status: ObservedStatus::Unknown,
        reported_result: None,
        freshness: "unknown".into(),
        association_status: AssociationStatus::Unverified,
        github_run_url: None,
        actions_job_id: None,
        workflow_run_attempt: None,
        github_conclusion: None,
        created_at: now,
        updated_at: now,
    };
    execute(tx, "INSERT INTO workflow_jobs(id,scope_key,fleet_key,fleet_incarnation,scale_set_id,
        protocol_job_id,summary_json,status,created_at,updated_at) VALUES(?,?,?,?,?,?,?,'unknown',?,?)",
        vec![summary.id.clone().into(), scope.key.clone().into(), fleet.into(), scope.incarnation.clone().into(),
            scope.scale_set_id.into(), protocol_id.into(), encode(&summary)?.into(), now.into(), now.into()]).await?;
    Ok(summary.id)
}
