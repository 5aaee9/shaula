use crate::entities::fleet::fleets;
use crate::entities::jobs::{
    forgejo_job_polls as polls, forgejo_workflow_jobs as forgejo_jobs,
    workflow_generation_identity as identity, workflow_jobs as github_jobs,
};
use crate::entities::lifecycle::{
    runner_generations as generations, runner_operations as operations,
};
use crate::entities::shared::audit_records as audit;
use crate::query_ext::{col, probe, same, JoinOn};
use crate::StoreResult;
use sea_orm::{
    sea_query::Expr, ColumnTrait, ConnectionTrait, EntityTrait, FromQueryResult, QueryFilter,
    QueryOrder, QuerySelect,
};
use shaula_core::{
    diagnostics::*,
    jobs::{AssociationStatus, JobSummary, ObservedStatus},
};

type View = (DiagnosticSubject, Vec<DiagnosticQuestion>, Option<Guard>);

fn ledger(question: QuestionId, observed: i64, revision: Option<i64>) -> DiagnosticQuestion {
    let mut q = DiagnosticQuestion::unavailable(question);
    q.basis.kind = BasisKind::LedgerProjection;
    q.basis.freshness = Freshness::Fresh;
    q.basis.observed_at = timestamp(observed);
    q.basis.subject_revision = revision.map(|v| v.to_string());
    q.coverage = Coverage::Partial;
    q
}

pub(super) async fn read(
    db: &impl ConnectionTrait,
    kind: SubjectKind,
    key: &str,
    now: i64,
) -> StoreResult<Option<View>> {
    match kind {
        SubjectKind::Fleet => fleet(db, key).await,
        SubjectKind::Generation => generation(db, key).await,
        SubjectKind::Job => job(db, key, now).await,
        SubjectKind::Unknown => Ok(None),
    }
}

async fn fleet(db: &impl ConnectionTrait, key: &str) -> StoreResult<Option<View>> {
    let row = fleets::Entity::find_by_id(key).one(db).await?;
    let Some(row) = row else { return Ok(None) };
    let guard = Guard {
        fleet_key: key.into(),
        incarnation: row.incarnation.clone(),
        revision: row.desired_revision,
        mutation_fence: row.mutation_fence,
        session_epoch: None,
        auth: None,
        generation_id: None,
        generation_updated_at: None,
        generation_revision: None,
        attempt_id: None,
        generation_pins: None,
    };
    let at = row.updated_at;
    let mut scale = DiagnosticQuestion::unavailable(QuestionId::ScaleUp);
    let mut rollout = DiagnosticQuestion::unavailable(QuestionId::Rollout);
    if row.deletion_marker || row.tombstone {
        scale = ledger(QuestionId::ScaleUp, at, Some(guard.revision));
        scale.reason(
            Code::ControlDecommissioning,
            StageId::Authority,
            Effect::Informational,
            EvidenceKind::LedgerFact,
        );
        scale.outcome = Outcome::NotApplicable;
        scale.coverage = Coverage::Complete;
        rollout = ledger(QuestionId::Rollout, at, Some(guard.revision));
        rollout.outcome = Outcome::NotApplicable;
        rollout.coverage = Coverage::Complete;
    }
    let counts = crate::generation_counts::fleet_counts(db, key).await?;
    let mut cleanup = ledger(QuestionId::Cleanup, at, Some(guard.revision));
    if counts.occupancy == 0 {
        cleanup.outcome = Outcome::Satisfied;
        cleanup.coverage = Coverage::Complete;
        cleanup.stage(StageId::Completion, Evaluation::Passed);
    } else if counts.quarantined > 0 {
        cleanup.reason(
            Code::CleanupQuarantined,
            StageId::Completion,
            Effect::Blocking,
            EvidenceKind::LedgerFact,
        );
    }
    Ok(Some((
        guard.subject(),
        vec![scale, cleanup, rollout],
        Some(guard),
    )))
}

/// One Generation joined to its recorded identity and, through that
/// identity's incarnation only, to the live Fleet head.
#[derive(FromQueryResult)]
struct GenerationRow {
    state: String,
    fleet_key: String,
    fleet_revision: i64,
    updated_at: i64,
    expiry_requested_at: Option<i64>,
    resources_destroyed_at: Option<i64>,
    fleet_incarnation: Option<String>,
    mutation_fence: Option<i64>,
    desired_revision: Option<i64>,
    finalized: bool,
}

async fn generation(db: &impl ConnectionTrait, key: &str) -> StoreResult<Option<View>> {
    let finalized = probe(audit::Entity)
        .and_where(col(audit::Column::ResourceKind).eq("generation"))
        .and_where(same(audit::Column::ResourceKey, generations::Column::Id))
        .and_where(col(audit::Column::Action).eq("finalize"))
        .to_owned();
    let row = generations::Entity::find_by_id(key)
        .select_only()
        .columns([
            generations::Column::State,
            generations::Column::FleetKey,
            generations::Column::FleetRevision,
            generations::Column::UpdatedAt,
            generations::Column::ExpiryRequestedAt,
            generations::Column::ResourcesDestroyedAt,
        ])
        .column(identity::Column::FleetIncarnation)
        .columns([
            fleets::Column::MutationFence,
            fleets::Column::DesiredRevision,
        ])
        .expr_as(Expr::exists(finalized), "finalized")
        .left_join_on(
            identity::Entity,
            same(identity::Column::GenerationId, generations::Column::Id),
        )
        .left_join_on(
            fleets::Entity,
            same(fleets::Column::Key, generations::Column::FleetKey).and(same(
                fleets::Column::Incarnation,
                identity::Column::FleetIncarnation,
            )),
        )
        .into_model::<GenerationRow>()
        .one(db)
        .await?;
    let Some(row) = row else { return Ok(None) };
    let incarnation = row.fleet_incarnation.clone();
    let revision = row.fleet_revision;
    let at = row.updated_at;
    let state = row.state.clone();
    let mut readiness = ledger(QuestionId::Readiness, at, Some(revision));
    let mut cleanup = ledger(QuestionId::Cleanup, at, Some(revision));
    match state.as_str() {
        "Destroyed" => {
            readiness.outcome = Outcome::NotApplicable;
            readiness.coverage = Coverage::Complete;
            cleanup.outcome = Outcome::Satisfied;
            cleanup.coverage = Coverage::Complete;
            cleanup.reason(
                Code::CleanupCompleted,
                StageId::Completion,
                Effect::Informational,
                EvidenceKind::LedgerFact,
            );
            cleanup.stage(StageId::Completion, Evaluation::Passed);
            let source = if row.finalized {
                CompletionSource::OperatorAttested
            } else {
                CompletionSource::Unknown
            };
            cleanup.reasons[0].parameters.completion_source = Some(source);
        }
        "Quarantined" => cleanup.reason(
            Code::CleanupQuarantined,
            StageId::Completion,
            Effect::Blocking,
            EvidenceKind::LedgerFact,
        ),
        _ => {}
    }
    if state != "Destroyed" && row.expiry_requested_at.is_some() {
        // Expiry intent is durable even when a restart loses runtime capture.
        // Keep the mode attached to that fact at both cleanup checkpoints.
        cleanup.cleanup_mode = Some(CleanupMode::HardLifetime);
        cleanup.reason(
            Code::CleanupHardLifetime,
            StageId::Intent,
            Effect::Informational,
            EvidenceKind::LedgerFact,
        );
    }
    if state != "Destroyed" && row.resources_destroyed_at.is_some() {
        cleanup.stage(StageId::ResourceCleanup, Evaluation::Passed);
        cleanup.reason(
            Code::CleanupRegistrationPending,
            StageId::RegistrationCleanup,
            Effect::Blocking,
            EvidenceKind::LedgerFact,
        );
    }
    let operation = operations::Entity::find()
        .select_only()
        .columns([operations::Column::Kind, operations::Column::State])
        .filter(operations::Column::GenerationId.eq(key))
        .order_by_desc(operations::Column::CreatedAt)
        .order_by_desc(operations::Column::Id)
        .limit(1)
        .into_tuple::<(String, String)>()
        .one(db)
        .await?;
    if let Some((kind, state)) = operation {
        operation_fact(&kind, &state, &mut readiness, &mut cleanup);
    }
    let subject = DiagnosticSubject {
        kind: SubjectKind::Generation,
        id: Some(key.into()),
        key: None,
        fleet_incarnation: incarnation.clone(),
    };
    let guard = match (incarnation, row.mutation_fence, row.desired_revision) {
        (Some(incarnation), Some(mutation_fence), Some(desired_revision)) => Some(Guard {
            fleet_key: row.fleet_key,
            incarnation,
            revision: desired_revision,
            mutation_fence,
            session_epoch: None,
            auth: None,
            generation_id: Some(key.into()),
            generation_updated_at: None,
            generation_revision: Some(revision),
            attempt_id: None,
            generation_pins: None,
        }),
        _ => None,
    };
    Ok(Some((subject, vec![readiness, cleanup], guard)))
}

fn operation_fact(
    kind: &str,
    state: &str,
    readiness: &mut DiagnosticQuestion,
    cleanup: &mut DiagnosticQuestion,
) {
    let q = if kind == "Destroy" {
        cleanup
    } else {
        readiness
    };
    if q.coverage == Coverage::Complete {
        return;
    }
    let stage = if kind == "Destroy" {
        StageId::ResourceCleanup
    } else {
        StageId::CreateEffect
    };
    match state {
        "BootstrapStarting" => {
            q.stage(StageId::CreateEffect, Evaluation::Passed);
            q.reason(
                Code::LifecycleBootstrapPending,
                StageId::Bootstrap,
                Effect::Informational,
                EvidenceKind::LedgerFact,
            );
        }
        "ApplyStarting" | "Applying" => q.reason(
            Code::LifecycleApplyOutcomeUnknown,
            stage,
            Effect::Informational,
            EvidenceKind::LedgerFact,
        ),
        "Failed" => q.reason(
            Code::LifecycleOperationFailed,
            stage,
            Effect::Blocking,
            EvidenceKind::LedgerFact,
        ),
        _ => {}
    }
}

/// A GitHub job summary, else a Forgejo one with its scope's last poll outcome.
async fn job_summary(
    db: &impl ConnectionTrait,
    key: &str,
) -> StoreResult<Option<(String, Option<bool>)>> {
    let github = github_jobs::Entity::find_by_id(key)
        .select_only()
        .column(github_jobs::Column::SummaryJson)
        .into_tuple::<String>()
        .one(db)
        .await?;
    if let Some(summary) = github {
        return Ok(Some((summary, None)));
    }
    Ok(forgejo_jobs::Entity::find_by_id(key)
        .select_only()
        .column(forgejo_jobs::Column::SummaryJson)
        .column(polls::Column::Failed)
        .left_join_on(
            polls::Entity,
            same(polls::Column::ScopeKey, forgejo_jobs::Column::ScopeKey),
        )
        .into_tuple::<(String, Option<bool>)>()
        .one(db)
        .await?)
}

async fn job(db: &impl ConnectionTrait, key: &str, now: i64) -> StoreResult<Option<View>> {
    let Some((summary_json, poll_failed)) = job_summary(db, key).await? else {
        return Ok(None);
    };
    let summary: JobSummary = serde_json::from_str(&summary_json)
        .map_err(|_| crate::StoreError::Corrupt("invalid job summary".into()))?;
    let mut q = ledger(QuestionId::JobDispatch, summary.updated_at, None);
    if summary.observed_status != ObservedStatus::Completed {
        q.basis.valid_until = timestamp(summary.updated_at.saturating_add(30_000));
    }
    q.stage(StageId::JobObservation, Evaluation::Passed);
    match summary.association_status {
        AssociationStatus::Unverified => q.reason(
            Code::JobAssociationUnverified,
            StageId::Association,
            Effect::Informational,
            EvidenceKind::LedgerFact,
        ),
        AssociationStatus::Ambiguous => q.reason(
            Code::JobAssociationAmbiguous,
            StageId::Association,
            Effect::Informational,
            EvidenceKind::LedgerFact,
        ),
        AssociationStatus::Verified => q.stage(StageId::Association, Evaluation::Passed),
    }
    q.reason(
        Code::JobDispatchNotObservable,
        StageId::ExecutionContext,
        Effect::Informational,
        EvidenceKind::LedgerFact,
    );
    if matches!(
        summary.observed_status,
        ObservedStatus::Running | ObservedStatus::Completed
    ) {
        q.outcome = Outcome::Satisfied;
    }
    if let Some(forgejo) = &summary.forgejo {
        if forgejo.result.is_none() {
            q.basis.observed_at = timestamp(forgejo.last_observed_at);
            q.basis.valid_until = timestamp(forgejo.last_observed_at.saturating_add(30_000));
            if !forgejo.in_snapshot || poll_failed != Some(false) {
                q.basis.freshness = Freshness::Stale;
                q.outcome = Outcome::Unknown;
                q.primary_reason_id = None;
                for evidence in &mut q.evidence {
                    evidence.freshness = Freshness::Stale;
                }
            }
        }
    }
    q.expire(now);
    Ok(Some((
        DiagnosticSubject {
            kind: SubjectKind::Job,
            id: Some(key.into()),
            key: None,
            fleet_incarnation: Some(summary.fleet_incarnation),
        },
        vec![q],
        None,
    )))
}
