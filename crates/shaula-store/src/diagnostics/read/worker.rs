//! Project only durable worker facts. No authority is reconstructed here and
//! no state bytes, capabilities, locks, process paths or inputs are exposed.
use crate::StoreResult;
use sea_orm::{ConnectionTrait, DatabaseBackend::Sqlite, Statement};
use shaula_core::diagnostics::*;

pub(super) async fn enrich(
    db: &impl ConnectionTrait,
    kind: SubjectKind,
    key: &str,
    questions: &mut [DiagnosticQuestion],
    now: i64,
) -> StoreResult<()> {
    if !matches!(kind, SubjectKind::Fleet | SubjectKind::Generation) {
        return Ok(());
    }
    let mode = db
        .query_one(Statement::from_string(
            Sqlite,
            "SELECT migration_required FROM lifecycle_deployment WHERE singleton = 1",
        ))
        .await?;
    if mode.is_some_and(|r| {
        r.try_get::<bool>("", "migration_required")
            .is_ok_and(|required| required)
    }) {
        for question in questions
            .iter_mut()
            .filter(|q| matches!(q.question, QuestionId::ScaleUp | QuestionId::Cleanup))
        {
            add(
                question,
                Code::WorkerMigrationRequired,
                Effect::Blocking,
                now,
            );
        }
    }
    if kind != SubjectKind::Generation {
        return Ok(());
    }
    let row = db.query_one(Statement::from_sql_and_values(Sqlite,
        "SELECT w.phase, w.handover IS NOT NULL AS handover, w.cleanup_only, w.completion_receipt IS NOT NULL AS receipt, s.state_bytes IS NOT NULL AS state_present, (SELECT CASE WHEN confirmed_fenced = 1 THEN 'fenced' ELSE outcome END FROM lifecycle_fences f WHERE f.worker_attempt = s.worker_attempt) AS fence FROM lifecycle_workers w JOIN generation_http_state s ON s.generation_id = w.generation_id WHERE w.generation_id = ?", vec![key.into()])).await?;
    let Some(row) = row else {
        return Ok(());
    };
    let Some(question) = questions
        .iter_mut()
        .find(|q| q.question == QuestionId::Cleanup)
    else {
        return Ok(());
    };
    if row.try_get::<bool>("", "receipt")? {
        add(
            question,
            Code::WorkerReceiptCommitted,
            Effect::Informational,
            now,
        );
        return Ok(());
    }
    if row.try_get::<String>("", "phase")? == "launch_pending"
        || row.try_get::<bool>("", "handover")?
    {
        add(
            question,
            Code::WorkerLaunchUnresolved,
            Effect::Blocking,
            now,
        );
    }
    if row.try_get::<Option<String>>("", "fence")?.as_deref() == Some("unknown") {
        add(question, Code::WorkerFencingUnknown, Effect::Blocking, now);
    }
    if !row.try_get::<bool>("", "state_present")? {
        add(
            question,
            Code::WorkerStateUnavailable,
            Effect::Blocking,
            now,
        );
    }
    if row.try_get::<bool>("", "cleanup_only")? {
        add(
            question,
            Code::WorkerCleanupOnly,
            Effect::Informational,
            now,
        );
    }
    Ok(())
}

fn add(question: &mut DiagnosticQuestion, code: Code, effect: Effect, now: i64) {
    // Each worker evidence item is current in this read transaction. Preserve
    // freshness of independent controller observations on the same question.
    let basis = question.basis.clone();
    question.basis.observed_at = timestamp(now);
    question.basis.freshness = Freshness::Fresh;
    let stage = if code == Code::WorkerMigrationRequired && question.question == QuestionId::Cleanup
    {
        StageId::ExecutionAdmission
    } else {
        code.definition().1
    };
    question.reason(code, stage, effect, EvidenceKind::LedgerFact);
    question.basis = basis;
}
