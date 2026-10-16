//! Retained message facts are committed in the listener's existing transaction.
use std::collections::BTreeSet;

use sea_orm::{
    sea_query::Expr, ActiveValue::Set, ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter,
    QuerySelect,
};
use shaula_core::jobs::{
    AssociationStatus, JobMetadata, JobObservation, JobSummary, ObservationKind, ObservedStatus,
};
use shaula_core::ports::PollMessage;
use shaula_core::registry::SessionEffectContext;

use super::{association, encode, scope};
use crate::entities::jobs::{workflow_job_observations as observations, workflow_jobs as jobs};
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
            observations::Entity::insert(observations::ActiveModel {
                id: Set(observation.id.clone()),
                scope_key: Set(scope.key.clone()),
                job_record_id: Set(job_id),
                protocol_job_id: Set(if fact.job.trim().is_empty() {
                    String::new()
                } else {
                    fact.job.into()
                }),
                runner_request_id: Set(fact.request),
                runner_id: Set(fact.runner),
                data_json: Set(encode(&observation)?),
                observed_at: Set(now),
            })
            .exec_without_returning(tx)
            .await?;
        }
        for request in requests {
            // Missing jobId can be joined only when all facts for this scoped
            // request agree. A later conflict retracts this tentative promotion.
            let candidates = observations::Entity::find()
                .select_only()
                .column(observations::Column::JobRecordId)
                .distinct()
                .filter(observations::Column::ScopeKey.eq(scope.key.as_str()))
                .filter(observations::Column::RunnerRequestId.eq(request))
                .filter(observations::Column::ProtocolJobId.ne(""))
                .filter(observations::Column::JobRecordId.is_not_null())
                .into_tuple::<String>()
                .all(tx)
                .await?;
            let candidate = if candidates.len() == 1 {
                Some(candidates[0].clone())
            } else {
                None
            };
            observations::Entity::update_many()
                .col_expr(observations::Column::JobRecordId, Expr::value(candidate))
                .filter(observations::Column::ScopeKey.eq(scope.key.as_str()))
                .filter(observations::Column::RunnerRequestId.eq(request))
                .filter(observations::Column::ProtocolJobId.eq(""))
                .exec(tx)
                .await?;
            affected.extend(candidates);
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
    let found = jobs::Entity::find()
        .select_only()
        .column(jobs::Column::Id)
        .filter(jobs::Column::ScopeKey.eq(scope.key.as_str()))
        .filter(jobs::Column::ProtocolJobId.eq(protocol_id))
        .into_tuple::<String>()
        .one(tx)
        .await?;
    if let Some(id) = found {
        return Ok(id);
    }
    let summary = JobSummary {
        id: uuid::Uuid::new_v4().to_string(),
        fleet_key: fleet.into(),
        fleet_incarnation: scope.incarnation.clone(),
        backend: shaula_core::fleet::FleetProviderKind::Github,
        forgejo: None,
        gitlab: None,
        scale_set_id: Some(scope.scale_set_id),
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
    jobs::Entity::insert(jobs::ActiveModel {
        id: Set(summary.id.clone()),
        scope_key: Set(scope.key.clone()),
        fleet_key: Set(fleet.into()),
        fleet_incarnation: Set(scope.incarnation.clone()),
        scale_set_id: Set(scope.scale_set_id),
        protocol_job_id: Set(protocol_id.into()),
        summary_json: Set(encode(&summary)?),
        status: Set("unknown".into()),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    })
    .exec_without_returning(tx)
    .await?;
    Ok(summary.id)
}
