use super::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    Supervisor,
    Assembly,
    Admission,
    Operation,
    Readiness,
    Cleanup,
    Follow,
}

/// Internal fences are never serialized into the public response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Guard {
    pub fleet_key: String,
    pub incarnation: String,
    pub revision: i64,
    pub mutation_fence: i64,
    pub session_epoch: Option<Option<i64>>,
    pub auth: Option<(String, i64)>,
    pub generation_id: Option<String>,
    pub generation_updated_at: Option<i64>,
    pub generation_revision: Option<i64>,
    pub attempt_id: Option<String>,
    #[serde(default)]
    pub generation_pins: Option<GenerationPins>,
}

impl Guard {
    pub fn fleet(key: &str, guard: &crate::registry::FleetRuntimeGuard) -> Self {
        Self {
            fleet_key: key.into(),
            incarnation: guard.incarnation.clone(),
            revision: guard.desired_revision,
            mutation_fence: guard.mutation_fence,
            session_epoch: None,
            auth: None,
            generation_id: None,
            generation_updated_at: None,
            generation_revision: None,
            attempt_id: None,
            generation_pins: None,
        }
    }
    pub fn bind_generation(&mut self, generation: &crate::registry::GenerationRecord) {
        self.generation_id = Some(generation.id.clone());
        self.generation_revision = Some(generation.fleet_revision);
        self.generation_pins = Some(GenerationPins {
            profile: generation.template_profile_key.clone(),
            revision: generation.template_revision,
            artifact: generation.template_artifact_digest.clone(),
            attestation: generation.attestation_id.clone(),
            inputs: generation.inputs_digest.clone(),
        });
    }
    pub fn subject(&self) -> DiagnosticSubject {
        DiagnosticSubject {
            kind: if self.generation_id.is_some() {
                SubjectKind::Generation
            } else {
                SubjectKind::Fleet
            },
            key: self.generation_id.is_none().then(|| self.fleet_key.clone()),
            id: self.generation_id.clone(),
            fleet_incarnation: Some(self.incarnation.clone()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionObservation {
    pub version: u8,
    pub guard: Guard,
    pub lane: Lane,
    pub epoch: String,
    pub sequence: u64,
    pub observed_at: i64,
    pub question: DiagnosticQuestion,
}

/// Optional and nonblocking. Failure cannot propagate into business operations.
pub trait DiagnosticSink: Send + Sync {
    fn register(&self, guard: &Guard, lane: Lane, question: QuestionId) -> Option<String>;
    fn sequence(&self, epoch: &str) -> Option<u64>;
    fn publish(&self, observation: DecisionObservation);
}

#[derive(Clone)]
pub struct Observer {
    sink: Arc<dyn DiagnosticSink>,
    guard: Guard,
    lane: Lane,
    question: QuestionId,
    epoch: String,
}
impl Observer {
    pub fn register(
        sink: Option<Arc<dyn DiagnosticSink>>,
        guard: Guard,
        lane: Lane,
        question: QuestionId,
    ) -> Option<Self> {
        let sink = sink?;
        let epoch = sink.register(&guard, lane, question)?;
        Some(Self {
            sink,
            guard,
            lane,
            question,
            epoch,
        })
    }
    /// Allocate before the first asynchronous operation of the observed work.
    pub fn begin(&self, now: i64) -> Option<ObservationTicket> {
        let sequence = self.sink.sequence(&self.epoch)?;
        let mut question = DiagnosticQuestion::unavailable(self.question);
        question.coverage = Coverage::Partial;
        question.basis = DiagnosticBasis {
            kind: BasisKind::RecordedDecision,
            observation_id: Some(format!("{}:{sequence}", self.epoch)),
            observed_at: timestamp(now),
            valid_until: now.checked_add(30_000).and_then(timestamp),
            freshness: Freshness::Fresh,
            subject_revision: Some(self.guard.revision.to_string()),
        };
        Some(ObservationTicket {
            sink: Arc::clone(&self.sink),
            observation: DecisionObservation {
                version: 1,
                guard: self.guard.clone(),
                lane: self.lane,
                epoch: self.epoch.clone(),
                sequence,
                observed_at: now,
                question,
            },
        })
    }
}

pub struct ObservationTicket {
    sink: Arc<dyn DiagnosticSink>,
    pub observation: DecisionObservation,
}
impl ObservationTicket {
    pub fn publish(self) {
        self.sink.publish(self.observation);
    }
}

/// Publishes on every return; unclassified paths remain explicitly unknown.
pub struct Capture(pub Option<ObservationTicket>);
impl Capture {
    pub fn start(
        sink: Option<Arc<dyn DiagnosticSink>>,
        guard: Guard,
        lane: Lane,
        question: QuestionId,
        now: i64,
    ) -> Self {
        Self(Observer::register(sink, guard, lane, question).and_then(|o| o.begin(now)))
    }
    pub fn reason(&mut self, code: Code, stage: StageId, blocking: bool) {
        if let Some(ticket) = &mut self.0 {
            ticket.observation.question.reason(
                code,
                stage,
                if blocking {
                    Effect::Blocking
                } else {
                    Effect::Informational
                },
                EvidenceKind::ControllerObservation,
            );
        }
    }
    pub fn pass(&mut self, stage: StageId) {
        if let Some(ticket) = &mut self.0 {
            ticket.observation.question.stage(stage, Evaluation::Passed);
        }
    }
    pub fn outcome(&mut self, outcome: Outcome) {
        if let Some(ticket) = &mut self.0 {
            ticket.observation.question.outcome = outcome;
        }
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        if let Some(ticket) = self.0.take() {
            ticket.publish();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationPins {
    pub profile: String,
    pub revision: i64,
    pub artifact: String,
    pub attestation: String,
    pub inputs: String,
}
