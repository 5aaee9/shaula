use shaula_core::diagnostics::*;
use std::collections::HashSet;

/// Global budgets preserve primary reasons and every retained reference.
pub(super) fn bound(report: &mut DiagnosticReportV1) {
    let mut remaining = 64usize;
    let mut primary_reservations = report
        .questions
        .iter()
        .filter(|q| q.primary_reason_id.is_some())
        .count();
    for q in &mut report.questions {
        primary_reservations =
            primary_reservations.saturating_sub(usize::from(q.primary_reason_id.is_some()));
        if q.pool.as_ref().is_some_and(|p| p.truncated) {
            report.truncated = true;
            q.coverage = Coverage::Partial;
        }
        // Reserve each question's primary first, then keep execution order.
        let primary = q.primary_reason_id.clone();
        let allowed = remaining
            .saturating_sub(primary_reservations)
            .min(q.reasons.len());
        if q.reasons.len() > allowed {
            let mut selected: HashSet<String> = primary.into_iter().take(allowed).collect();
            for reason in &q.reasons {
                if selected.len() >= allowed {
                    break;
                }
                selected.insert(reason.id.clone());
            }
            q.reasons.retain(|r| selected.contains(&r.id));
            q.coverage = Coverage::Partial;
            report.truncated = true;
        }
        remaining = remaining.saturating_sub(q.reasons.len());
        prune(q);
    }
    if report.related.len() > 32 {
        report.related.truncate(32);
        report.truncated = true;
    }
    // Each retained reason currently has one evidence item; enforce the wire
    // limit independently so future producers cannot accidentally exceed it.
    let mut evidence_budget = 128;
    for q in &mut report.questions {
        if q.evidence.len() > evidence_budget {
            q.evidence.truncate(evidence_budget);
            q.coverage = Coverage::Partial;
            report.truncated = true;
        }
        evidence_budget = evidence_budget.saturating_sub(q.evidence.len());
        prune(q);
    }
    while serde_json::to_vec(report).is_ok_and(|v| v.len() > 256 * 1024) {
        report.truncated = true;
        if let Some(q) = report.questions.iter_mut().rev().find(|q| {
            q.reasons
                .iter()
                .any(|r| Some(&r.id) != q.primary_reason_id.as_ref())
        }) {
            if let Some(index) = q
                .reasons
                .iter()
                .rposition(|r| Some(&r.id) != q.primary_reason_id.as_ref())
            {
                q.reasons.remove(index);
            }
            q.coverage = Coverage::Partial;
            prune(q);
        } else {
            // Pathological optional data may not displace the primary cause.
            for q in &mut report.questions {
                q.capacity = None;
                q.pool = None;
                q.rollout = None;
                q.suggestions.clear();
                q.coverage = Coverage::Partial;
            }
            report.related.clear();
            break;
        }
    }
}

fn prune(q: &mut DiagnosticQuestion) {
    let reasons: HashSet<_> = q.reasons.iter().map(|r| r.id.as_str()).collect();
    for stage in &mut q.stages {
        stage.reason_ids.retain(|id| reasons.contains(id.as_str()));
    }
    if q.primary_reason_id
        .as_ref()
        .is_some_and(|id| !reasons.contains(id.as_str()))
    {
        q.primary_reason_id = None;
        q.outcome = Outcome::Unknown;
    }
    let referenced: HashSet<_> = q
        .reasons
        .iter()
        .flat_map(|r| r.evidence_ids.iter())
        .collect();
    q.evidence.retain(|e| referenced.contains(&e.id));
    let evidence: HashSet<_> = q.evidence.iter().map(|e| e.id.as_str()).collect();
    for reason in &mut q.reasons {
        reason
            .evidence_ids
            .retain(|id| evidence.contains(id.as_str()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn global_budget_keeps_all_primary_causes_and_references() -> Result<(), serde_json::Error> {
        let mut questions = Vec::new();
        for question in [
            QuestionId::ScaleUp,
            QuestionId::Cleanup,
            QuestionId::Rollout,
        ] {
            let mut q = DiagnosticQuestion::unavailable(question);
            q.basis.freshness = Freshness::Fresh;
            for _ in 0..100 {
                q.reason(
                    Code::ObservationMissing,
                    question.stages()[0],
                    Effect::Informational,
                    EvidenceKind::LedgerFact,
                );
            }
            q.reason(
                Code::ObservationConflict,
                question.stages()[0],
                Effect::Blocking,
                EvidenceKind::LedgerFact,
            );
            questions.push(q);
        }
        let mut report = DiagnosticReportV1 {
            schema_version: 1,
            subject: DiagnosticSubject {
                kind: SubjectKind::Fleet,
                key: Some("fleet".into()),
                id: None,
                fleet_incarnation: Some("inc".into()),
            },
            generated_at: timestamp(0).unwrap_or_default(),
            questions,
            related: vec![],
            truncated: false,
        };
        bound(&mut report);
        assert!(report.truncated);
        assert!(
            report
                .questions
                .iter()
                .map(|q| q.reasons.len())
                .sum::<usize>()
                <= 64
        );
        assert!(serde_json::to_vec(&report)?.len() <= 256 * 1024);
        for q in &report.questions {
            assert!(q
                .reasons
                .iter()
                .any(|r| Some(&r.id) == q.primary_reason_id.as_ref()));
            for reason in &q.reasons {
                for id in &reason.evidence_ids {
                    assert!(q.evidence.iter().any(|e| &e.id == id));
                }
            }
        }
        Ok(())
    }
}
