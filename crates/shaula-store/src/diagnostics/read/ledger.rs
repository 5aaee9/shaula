use crate::StoreResult;
use sea_orm::{ConnectionTrait, DatabaseBackend::Sqlite, QueryResult, Statement};
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
    let row = db.query_one(Statement::from_sql_and_values(Sqlite,
        "SELECT incarnation,desired_revision,mutation_fence,deletion_marker,tombstone,updated_at FROM fleets WHERE key=?", vec![key.into()])).await?;
    let Some(row) = row else { return Ok(None) };
    let guard = Guard {
        fleet_key: key.into(),
        incarnation: row.try_get("", "incarnation")?,
        revision: row.try_get("", "desired_revision")?,
        mutation_fence: row.try_get("", "mutation_fence")?,
        session_epoch: None,
        auth: None,
        generation_id: None,
        generation_updated_at: None,
        generation_revision: None,
        attempt_id: None,
        generation_pins: None,
    };
    let at = row.try_get("", "updated_at")?;
    let mut scale = DiagnosticQuestion::unavailable(QuestionId::ScaleUp);
    let mut rollout = DiagnosticQuestion::unavailable(QuestionId::Rollout);
    if row.try_get::<bool>("", "deletion_marker")? || row.try_get::<bool>("", "tombstone")? {
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
    let row = db.query_one(Statement::from_sql_and_values(Sqlite,
        "SELECT COUNT(*) AS occupancy,COALESCE(SUM(state='Quarantined'),0) AS quarantined FROM runner_generations WHERE fleet_key=? AND state!='Destroyed'",
        vec![key.into()])).await?;
    let mut cleanup = ledger(QuestionId::Cleanup, at, Some(guard.revision));
    if let Some(row) = row {
        if row.try_get::<i64>("", "occupancy")? == 0 {
            cleanup.outcome = Outcome::Satisfied;
            cleanup.coverage = Coverage::Complete;
            cleanup.stage(StageId::Completion, Evaluation::Passed);
        } else if row.try_get::<i64>("", "quarantined")? > 0 {
            cleanup.reason(
                Code::CleanupQuarantined,
                StageId::Completion,
                Effect::Blocking,
                EvidenceKind::LedgerFact,
            );
        }
    }
    Ok(Some((
        guard.subject(),
        vec![scale, cleanup, rollout],
        Some(guard),
    )))
}

async fn generation(db: &impl ConnectionTrait, key: &str) -> StoreResult<Option<View>> {
    let row = db.query_one(Statement::from_sql_and_values(Sqlite,
        "SELECT g.state,g.fleet_key,g.fleet_revision,g.updated_at,g.expiry_requested_at,g.resources_destroyed_at,
         i.fleet_incarnation,f.mutation_fence,f.desired_revision,
         EXISTS(SELECT 1 FROM audit_records a WHERE a.resource_kind='generation' AND a.resource_key=g.id AND a.action='finalize') AS finalized
         FROM runner_generations g LEFT JOIN workflow_generation_identity i ON i.generation_id=g.id
         LEFT JOIN fleets f ON f.key=g.fleet_key AND f.incarnation=i.fleet_incarnation WHERE g.id=?", vec![key.into()])).await?;
    let Some(row) = row else { return Ok(None) };
    let incarnation: Option<String> = row.try_get("", "fleet_incarnation")?;
    let revision = row.try_get("", "fleet_revision")?;
    let at = row.try_get("", "updated_at")?;
    let state: String = row.try_get("", "state")?;
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
            let source = if row.try_get::<bool>("", "finalized")? {
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
    if state != "Destroyed"
        && row
            .try_get::<Option<i64>>("", "expiry_requested_at")?
            .is_some()
    {
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
    if state != "Destroyed"
        && row
            .try_get::<Option<i64>>("", "resources_destroyed_at")?
            .is_some()
    {
        cleanup.stage(StageId::ResourceCleanup, Evaluation::Passed);
        cleanup.reason(
            Code::CleanupRegistrationPending,
            StageId::RegistrationCleanup,
            Effect::Blocking,
            EvidenceKind::LedgerFact,
        );
    }
    let operation = db.query_one(Statement::from_sql_and_values(Sqlite,
        "SELECT state,kind,updated_at FROM runner_operations WHERE generation_id=? ORDER BY created_at DESC,id DESC LIMIT 1", vec![key.into()])).await?;
    if let Some(op) = operation {
        operation_fact(&op, &mut readiness, &mut cleanup)?;
    }
    let subject = DiagnosticSubject {
        kind: SubjectKind::Generation,
        id: Some(key.into()),
        key: None,
        fleet_incarnation: incarnation.clone(),
    };
    let fence: Option<i64> = row.try_get("", "mutation_fence")?;
    let guard = match (incarnation, fence) {
        (Some(incarnation), Some(mutation_fence)) => Some(Guard {
            fleet_key: row.try_get("", "fleet_key")?,
            incarnation,
            revision: row.try_get("", "desired_revision")?,
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
    row: &QueryResult,
    readiness: &mut DiagnosticQuestion,
    cleanup: &mut DiagnosticQuestion,
) -> StoreResult<()> {
    let kind: String = row.try_get("", "kind")?;
    let q = if kind == "Destroy" {
        cleanup
    } else {
        readiness
    };
    if q.coverage == Coverage::Complete {
        return Ok(());
    }
    let stage = if kind == "Destroy" {
        StageId::ResourceCleanup
    } else {
        StageId::CreateEffect
    };
    let state: String = row.try_get("", "state")?;
    match state.as_str() {
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
    Ok(())
}

async fn job(db: &impl ConnectionTrait, key: &str, now: i64) -> StoreResult<Option<View>> {
    let row = db.query_one(Statement::from_sql_and_values(Sqlite,
        "SELECT summary_json,NULL AS poll_time,NULL AS poll_failed FROM workflow_jobs WHERE id=? UNION ALL SELECT j.summary_json,p.observed_at AS poll_time,p.failed AS poll_failed FROM forgejo_workflow_jobs j LEFT JOIN forgejo_job_polls p ON p.scope_key=j.scope_key WHERE j.id=? LIMIT 1", vec![key.into(),key.into()])).await?;
    let Some(row) = row else { return Ok(None) };
    let summary: JobSummary = serde_json::from_str(&row.try_get::<String>("", "summary_json")?)
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
            if !forgejo.in_snapshot || row.try_get::<Option<i64>>("", "poll_failed")? != Some(0) {
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
