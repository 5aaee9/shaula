use super::*;
use crate::capacity::{create_count, target, AssignedDemand, CapacityCounters, CapacityPolicy};

pub fn timestamp(ms: i64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

pub fn capacity(
    policy: CapacityPolicy,
    demand: Option<i64>,
    counters: CapacityCounters,
    kind: DemandKind,
) -> DiagnosticCapacity {
    let assigned = demand.map(|total_assigned_jobs| AssignedDemand {
        total_assigned_jobs,
    });
    let desired = assigned.as_ref().map(|d| target(&policy, d));
    DiagnosticCapacity {
        demand_kind: kind,
        min: policy.min_runners.to_string(),
        max: policy.max_runners.to_string(),
        demand: demand.map(|v| v.to_string()),
        target: desired.map(|v| v.to_string()),
        effective: counters.effective_capacity.to_string(),
        occupancy: counters.resource_occupancy.to_string(),
        deficit: desired.map(|v| {
            v.saturating_sub(counters.effective_capacity)
                .max(0)
                .to_string()
        }),
        occupancy_headroom: policy
            .max_runners
            .saturating_sub(counters.resource_occupancy)
            .max(0)
            .to_string(),
        arithmetic_create_allowance: assigned
            .as_ref()
            .map(|d| create_count(&policy, d, &counters).to_string()),
        actually_admitted: None,
    }
}

pub trait QuestionStages {
    fn stages(self) -> &'static [StageId];
}
impl QuestionStages for QuestionId {
    fn stages(self) -> &'static [StageId] {
        use StageId::*;
        match self {
            Self::ScaleUp => &[Authority, Demand, Capacity, Pool, ExecutionAdmission],
            Self::Readiness => &[CreateEffect, Bootstrap, RunnerInventory],
            Self::Cleanup => &[
                Intent,
                SafeRemoval,
                ExecutionAdmission,
                ResourceCleanup,
                RegistrationCleanup,
                Completion,
            ],
            Self::Rollout => &[
                DependencyResolution,
                InputCompatibility,
                Occupancy,
                CommitFence,
            ],
            Self::JobDispatch => &[JobObservation, Association, ExecutionContext],
            Self::Unknown => &[],
        }
    }
}

pub trait QuestionExt {
    fn unavailable(question: QuestionId) -> Self;
    fn stage(&mut self, id: StageId, evaluation: Evaluation);
    fn reason(&mut self, code: Code, stage: StageId, effect: Effect, kind: EvidenceKind);
    fn expire(&mut self, now: i64);
}
impl QuestionExt for DiagnosticQuestion {
    fn unavailable(question: QuestionId) -> Self {
        Self {
            question,
            outcome: Outcome::Unknown,
            coverage: Coverage::None,
            basis: DiagnosticBasis {
                kind: BasisKind::Unavailable,
                observation_id: None,
                observed_at: None,
                valid_until: None,
                freshness: Freshness::Missing,
                subject_revision: None,
            },
            stages: question
                .stages()
                .iter()
                .map(|id| DiagnosticStage {
                    id: *id,
                    evaluation: Evaluation::NotEvaluated,
                    reason_ids: vec![],
                })
                .collect(),
            primary_reason_id: None,
            reasons: vec![],
            evidence: vec![],
            suggestions: vec![],
            capacity: None,
            pool: None,
            rollout: None,
            cleanup_mode: None,
        }
    }

    fn stage(&mut self, id: StageId, evaluation: Evaluation) {
        if let Some(stage) = self.stages.iter_mut().find(|s| s.id == id) {
            stage.evaluation = evaluation;
        }
    }

    fn reason(&mut self, code: Code, stage: StageId, effect: Effect, kind: EvidenceKind) {
        let (literal, _, severity, _) = code.definition();
        let id = format!("r-{}", self.reasons.len());
        let evidence_id = format!("e-{}", self.evidence.len());
        let at = self.basis.observed_at.clone();
        self.evidence.push(DiagnosticEvidence {
            id: evidence_id.clone(),
            kind,
            observed_at: at.clone(),
            freshness: self.basis.freshness,
            valid_until: self.basis.valid_until.clone(),
            predicate: literal.into(),
        });
        if let Some(view) = self.stages.iter_mut().find(|s| s.id == stage) {
            view.reason_ids.push(id.clone());
        }
        if effect == Effect::Blocking && self.basis.freshness == Freshness::Fresh {
            self.stage(stage, Evaluation::Blocked);
            self.outcome = Outcome::Blocked;
            self.primary_reason_id.get_or_insert(id.clone());
        }
        if effect == Effect::Informational && self.basis.freshness == Freshness::Fresh {
            let evaluated = match code {
                Code::LifecycleBootstrapPending
                | Code::LifecycleAwaitingOnline
                | Code::ExecutionCreateSlotWait
                | Code::ExecutionDestroySlotWait => Some(Evaluation::Pending),
                Code::LifecycleApplyOutcomeUnknown
                | Code::JobDispatchNotObservable
                | Code::JobAssociationUnverified
                | Code::JobAssociationAmbiguous
                | Code::ControlDemandUnavailable
                | Code::ControlInventoryUnavailable => Some(Evaluation::Unknown),
                Code::CleanupNoCreateEffect | Code::ControlDecommissioning => {
                    Some(Evaluation::NotApplicable)
                }
                Code::CleanupCompleted | Code::CleanupHardLifetime => Some(Evaluation::Passed),
                _ => None,
            };
            if let Some(evaluation) = evaluated {
                self.stage(stage, evaluation);
            }
        }
        self.reasons.push(DiagnosticReason {
            id,
            code: literal.into(),
            stage,
            severity,
            effect,
            parameters: DiagnosticParameters::default(),
            evidence_ids: vec![evidence_id],
            first_observed_at: at.clone(),
            last_observed_at: at,
        });
    }

    fn expire(&mut self, now: i64) {
        let observed = self
            .basis
            .observed_at
            .as_deref()
            .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
            .map(|v| v.timestamp_millis());
        let until = self
            .basis
            .valid_until
            .as_deref()
            .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
            .map(|v| v.timestamp_millis());
        let clock_invalid = observed.is_some_and(|v| v > now);
        if clock_invalid || until.is_some_and(|v| now >= v) {
            self.outcome = Outcome::Unknown;
            self.coverage = Coverage::Partial;
            self.primary_reason_id = None;
            self.basis.freshness = if clock_invalid {
                Freshness::Unknown
            } else {
                Freshness::Stale
            };
            for evidence in &mut self.evidence {
                evidence.freshness = self.basis.freshness;
            }
            for stage in &mut self.stages {
                if stage.evaluation != Evaluation::NotEvaluated {
                    stage.evaluation = Evaluation::Unknown;
                }
            }
            let code = if clock_invalid {
                Code::ObservationClockInvalid
            } else {
                Code::ObservationStale
            };
            if self.reasons.iter().any(|r| r.code == code.as_str()) {
                return;
            }
            self.reason(
                code,
                self.question
                    .stages()
                    .first()
                    .copied()
                    .unwrap_or(StageId::Authority),
                Effect::Informational,
                EvidenceKind::ControllerObservation,
            );
        }
    }
}
