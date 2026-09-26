use crate::Store;
use sea_orm::{ConnectionTrait, DatabaseBackend::Sqlite, Statement, TransactionTrait};
use shaula_core::{
    diagnostics::*,
    registry::{Actor, Scope},
};

mod budget;
mod ledger;

#[async_trait::async_trait]
impl DiagnosticsReadPort for Store {
    async fn diagnostics(
        &self,
        kind: SubjectKind,
        key: &str,
        actor: &Actor,
        now: i64,
    ) -> DiagnosticsResult {
        if !actor.has(Scope::FleetRead) {
            return Err(unavailable());
        }
        self.read_diagnostics(kind, key, actor, now)
            .await
            .map_err(|e| match e {
                ReadFailure::Gone => DiagnosticsReadError::Gone,
                _ => unavailable(),
            })
    }
}
fn unavailable() -> DiagnosticsReadError {
    DiagnosticsReadError::Unavailable
}
#[derive(Debug, thiserror::Error)]
enum ReadFailure {
    #[error("gone")]
    Gone,
    #[error(transparent)]
    Store(#[from] crate::StoreError),
    #[error(transparent)]
    Database(#[from] sea_orm::DbErr),
}

impl Store {
    async fn read_diagnostics(
        &self,
        kind: SubjectKind,
        key: &str,
        actor: &Actor,
        now: i64,
    ) -> Result<Option<DiagnosticReportV1>, ReadFailure> {
        let tx = self.connection().begin().await?;
        if kind == SubjectKind::Fleet {
            if let Some(row) = tx
                .query_one(Statement::from_sql_and_values(
                    Sqlite,
                    "SELECT tombstone FROM fleets WHERE key=?",
                    vec![key.into()],
                ))
                .await?
            {
                if row.try_get::<bool>("", "tombstone")? {
                    return Err(ReadFailure::Gone);
                }
            }
        }

        let Some((mut subject, mut questions, mut guard)) =
            ledger::read(&tx, kind, key, now).await?
        else {
            return Ok(None);
        };
        // Forgejo has no GitHub workflow identity. Only a captured exact
        // Generation identity may fill this gap; never join by Fleet name.
        if guard.is_none() && kind == SubjectKind::Generation {
            if let Ok(Some(row)) = tx.query_one(Statement::from_sql_and_values(Sqlite,
                "SELECT payload FROM diagnostic_snapshots WHERE json_extract(payload,'$.guard.generation_id')=? ORDER BY observed_at DESC LIMIT 1", vec![key.into()])).await {
                if let Ok(payload) = row.try_get::<String>("", "payload") {
                    if let Ok(o) = serde_json::from_str::<DecisionObservation>(&payload) {
                        if o.version == 1 && super::guard_valid(&tx, &o.guard).await? {
                            subject.fleet_incarnation = Some(o.guard.incarnation.clone());
                            guard = Some(o.guard);
                        }
                    }
                }
            }
        }
        let mut truncated = false;
        if let Some(guard) = guard {
            // A bounded set of fixed lanes; no Generation/Job table scan per GET.
            let rows = tx.query_all(Statement::from_sql_and_values(Sqlite,
                "SELECT payload FROM diagnostic_snapshots WHERE subject_key=? ORDER BY question,lane LIMIT 22",
                vec![super::subject_key(&guard).into()])).await;
            // Optional projection failure leaves the independently read ledger usable.
            if let Ok(rows) = rows {
                truncated = rows.len() > 21;
                for row in rows.into_iter().take(21) {
                    let Ok(payload) = row.try_get::<String>("", "payload") else {
                        continue;
                    };
                    let Ok(mut o) = serde_json::from_str::<DecisionObservation>(&payload) else {
                        continue;
                    };
                    if o.version != 1 || !super::guard_valid(&tx, &o.guard).await? {
                        continue;
                    }
                    if !self.diagnostics.current(&o) {
                        o.question.outcome = Outcome::Unknown;
                        o.question.coverage = Coverage::Partial;
                        o.question.primary_reason_id = None;
                        for stage in &mut o.question.stages {
                            stage.evaluation = Evaluation::Unknown;
                        }
                        o.question.basis.freshness = Freshness::Missing;
                        o.question.reason(
                            self.diagnostics.invalid_reason(&o.epoch),
                            o.question
                                .question
                                .stages()
                                .first()
                                .copied()
                                .unwrap_or(StageId::Authority),
                            Effect::Informational,
                            EvidenceKind::ControllerObservation,
                        );
                        for e in &mut o.question.evidence {
                            e.freshness = Freshness::Missing;
                        }
                    }
                    o.question.expire(now);
                    if let Some(q) = questions
                        .iter_mut()
                        .find(|q| q.question == o.question.question)
                    {
                        merge(q, o.question);
                    }
                }
            }
        }
        for q in &mut questions {
            if q.reasons.is_empty() && q.outcome == Outcome::Unknown {
                q.reason(
                    Code::ObservationMissing,
                    q.question
                        .stages()
                        .first()
                        .copied()
                        .unwrap_or(StageId::Authority),
                    Effect::Informational,
                    EvidenceKind::ControllerObservation,
                );
            }
            q.reasons.sort_by_key(|r| {
                (
                    q.question
                        .stages()
                        .iter()
                        .position(|s| *s == r.stage)
                        .unwrap_or(usize::MAX),
                    r.code.clone(),
                )
            });
            if q.basis.freshness == Freshness::Fresh {
                q.primary_reason_id = q
                    .reasons
                    .iter()
                    .find(|r| r.effect == Effect::Blocking)
                    .map(|r| r.id.clone());
            }
            q.expire(now);
            if !actor.has(Scope::TemplateRead) {
                q.pool = None;
                if let Some(rollout) = &mut q.rollout {
                    rollout.previous_pin = None;
                    rollout.candidate_pin = None;
                }
            }
            let suggestion = match subject.kind {
                SubjectKind::Fleet => Some(SuggestionId::ViewGenerations),
                SubjectKind::Generation if actor.has(Scope::LogsRead) => {
                    Some(SuggestionId::ViewInvocations)
                }
                _ => None,
            };
            if let Some(suggestion_id) = suggestion {
                q.suggestions = vec![DiagnosticSuggestion {
                    suggestion_id,
                    reference: Some(subject.clone()),
                }];
            }
            if truncated {
                q.coverage = Coverage::Partial;
            }
        }
        let mut report = DiagnosticReportV1 {
            schema_version: 1,
            subject,
            generated_at: timestamp(now)
                .ok_or_else(|| crate::StoreError::Corrupt("invalid diagnostic clock".into()))?,
            questions,
            related: vec![],
            truncated,
        };
        budget::bound(&mut report);
        Ok(Some(report))
    }
}

fn merge(current: &mut DiagnosticQuestion, mut next: DiagnosticQuestion) {
    // A durable terminal ledger predicate is authoritative. Runtime evidence
    // never downgrades it or changes its completion source.
    if current.coverage == Coverage::Complete && current.basis.kind == BasisKind::LedgerProjection {
        if current.question == QuestionId::Cleanup {
            let source = next
                .reasons
                .iter()
                .find(|r| r.code == Code::CleanupCompleted.as_str())
                .and_then(|r| r.parameters.completion_source);
            if let Some(source) = source {
                for reason in &mut current.reasons {
                    if reason.code == Code::CleanupCompleted.as_str()
                        && reason.parameters.completion_source == Some(CompletionSource::Unknown)
                    {
                        reason.parameters.completion_source = Some(source);
                    }
                }
            }
        }
        return;
    }
    if current.basis.kind == BasisKind::Unavailable {
        *current = next;
        return;
    }
    if next.basis.freshness != Freshness::Fresh {
        return;
    }
    if current.basis.kind == BasisKind::LedgerProjection
        || current.basis.freshness != Freshness::Fresh
    {
        *current = next;
        return;
    }
    // Different lanes are independent evidence, never a synthetic atomic capacity
    // decision. Preserve each reason's own timestamps and stable remapped IDs.
    let prefix = format!("lane{}-", current.reasons.len());
    for evidence in &mut next.evidence {
        evidence.id = format!("{prefix}{}", evidence.id);
    }
    for reason in &mut next.reasons {
        reason.id = format!("{prefix}{}", reason.id);
        for id in &mut reason.evidence_ids {
            *id = format!("{prefix}{id}");
        }
    }
    for stage in &mut next.stages {
        for id in &mut stage.reason_ids {
            *id = format!("{prefix}{id}");
        }
        if stage.evaluation != Evaluation::NotEvaluated {
            if let Some(current) = current.stages.iter_mut().find(|s| s.id == stage.id) {
                *current = stage.clone();
            }
        }
    }
    if next.outcome == Outcome::Blocked {
        current.outcome = Outcome::Blocked;
        if current.primary_reason_id.is_none() {
            current.primary_reason_id = next.primary_reason_id.map(|v| format!("{prefix}{v}"));
        }
    }
    if next.capacity.is_some() {
        current.capacity = next.capacity;
    }
    if next.pool.is_some() {
        current.pool = next.pool;
    }
    if next.rollout.is_some() {
        current.rollout = next.rollout;
    }
    if next.cleanup_mode.is_some() {
        current.cleanup_mode = next.cleanup_mode;
    }
    current.reasons.extend(next.reasons);
    current.evidence.extend(next.evidence);
    current.coverage = Coverage::Partial;
}
