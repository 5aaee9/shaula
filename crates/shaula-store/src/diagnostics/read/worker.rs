//! Project only durable worker facts. No authority is reconstructed here and
//! no state bytes, capabilities, locks, process paths or inputs are exposed.
use crate::entities::http_state::{
    generation_http_state as states, lifecycle_deployment as deployment,
    lifecycle_fences as fences, lifecycle_workers as workers,
};
use crate::query_ext::{col, same, scalar, JoinOn};
use crate::StoreResult;
use sea_orm::{
    sea_query::{Expr, Query},
    ConnectionTrait, EntityTrait, QuerySelect,
};
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
    let migration_required = deployment::Entity::find_by_id(1)
        .select_only()
        .column(deployment::Column::MigrationRequired)
        .into_tuple::<bool>()
        .one(db)
        .await?;
    if migration_required == Some(true) {
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
    let fence = Query::select()
        .expr(
            Expr::case(col(fences::Column::ConfirmedFenced).eq(true), "fenced")
                .finally(col(fences::Column::Outcome)),
        )
        .from(fences::Entity)
        .and_where(same(
            fences::Column::WorkerAttempt,
            states::Column::WorkerAttempt,
        ))
        .to_owned();
    let row = workers::Entity::find_by_id(key)
        .select_only()
        .column(workers::Column::Phase)
        .expr(col(workers::Column::Handover).is_not_null())
        .column(workers::Column::CleanupOnly)
        .expr(col(workers::Column::CompletionReceipt).is_not_null())
        .expr(col(states::Column::StateBytes).is_not_null())
        .expr(scalar(fence))
        .inner_join_on(
            states::Entity,
            same(states::Column::GenerationId, workers::Column::GenerationId),
        )
        .into_tuple::<(String, bool, bool, bool, bool, Option<String>)>()
        .one(db)
        .await?;
    let Some((phase, handover, cleanup_only, receipt, state_present, fence)) = row else {
        return Ok(());
    };
    let Some(question) = questions
        .iter_mut()
        .find(|q| q.question == QuestionId::Cleanup)
    else {
        return Ok(());
    };
    if receipt {
        add(
            question,
            Code::WorkerReceiptCommitted,
            Effect::Informational,
            now,
        );
        return Ok(());
    }
    if phase == "launch_pending" || handover {
        add(
            question,
            Code::WorkerLaunchUnresolved,
            Effect::Blocking,
            now,
        );
    }
    if fence.as_deref() == Some("unknown") {
        add(question, Code::WorkerFencingUnknown, Effect::Blocking, now);
    }
    if !state_present {
        add(
            question,
            Code::WorkerStateUnavailable,
            Effect::Blocking,
            now,
        );
    }
    if cleanup_only {
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
