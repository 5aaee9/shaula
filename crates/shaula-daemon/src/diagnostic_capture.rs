//! Small helpers for actual branch observations, not a shadow controller.
use shaula_core::diagnostics::*;

pub(crate) fn note(
    ticket: &mut Option<ObservationTicket>,
    code: Code,
    stage: StageId,
    blocking: bool,
) {
    if let Some(ticket) = ticket {
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
pub(crate) fn passed(ticket: &mut Option<ObservationTicket>, stage: StageId) {
    if let Some(ticket) = ticket {
        ticket.observation.question.stage(stage, Evaluation::Passed);
    }
}
pub(crate) fn capacity_decision(
    ticket: &mut Option<ObservationTicket>,
    policy: shaula_core::capacity::CapacityPolicy,
    demand: Option<i64>,
    counters: shaula_core::capacity::CapacityCounters,
    kind: DemandKind,
    successful_at: Option<i64>,
) {
    let Some(ticket) = ticket else { return };
    let q = &mut ticket.observation.question;
    q.capacity = Some(capacity(policy, demand, counters, kind));
    q.stage(StageId::Capacity, Evaluation::Passed);
    if demand.is_none() || successful_at.is_none() {
        q.stage(StageId::Capacity, Evaluation::Unknown);
        q.reason(
            Code::ControlDemandUnavailable,
            StageId::Demand,
            Effect::Informational,
            EvidenceKind::ControllerObservation,
        );
        return;
    }
    if let Some(at) = successful_at {
        q.basis.valid_until = timestamp(
            at.saturating_add(30_000)
                .min(ticket.observation.observed_at.saturating_add(30_000)),
        );
        if at > ticket.observation.observed_at {
            q.reason(
                Code::ObservationClockInvalid,
                StageId::Demand,
                Effect::Informational,
                EvidenceKind::ControllerObservation,
            );
            q.basis.freshness = Freshness::Unknown;
            return;
        }
    }
    let cap = q.capacity.as_ref();
    if cap.is_some_and(|v| v.deficit.as_deref() == Some("0")) {
        q.outcome = Outcome::Satisfied;
        q.reason(
            Code::CapacityTargetSatisfied,
            StageId::Capacity,
            Effect::Informational,
            EvidenceKind::DerivedCalculation,
        );
    } else if cap.is_some_and(|v| v.occupancy_headroom == "0") {
        q.reason(
            Code::CapacityOccupancyLimit,
            StageId::Capacity,
            Effect::Blocking,
            EvidenceKind::DerivedCalculation,
        );
    }
    if demand.is_some_and(|d| policy.min_runners.saturating_add(d) > policy.max_runners) {
        q.reason(
            Code::CapacityPolicyCeiling,
            StageId::Capacity,
            Effect::Informational,
            EvidenceKind::DerivedCalculation,
        );
    }
    q.expire(ticket.observation.observed_at);
}

pub(crate) fn finish(mut ticket: Option<ObservationTicket>, failed: bool) {
    if failed {
        note(
            &mut ticket,
            Code::ObservationUnclassifiedFailure,
            StageId::ExecutionAdmission,
            false,
        );
    }
    if let Some(ticket) = ticket {
        ticket.publish();
    }
}

pub(crate) fn generation_capture(
    store: &dyn shaula_core::registry::ControlPlaneStore,
    runtime_guard: Option<&shaula_core::registry::FleetRuntimeGuard>,
    generation: &shaula_core::registry::GenerationRecord,
    lane: Lane,
    question: QuestionId,
    now: i64,
) -> Capture {
    let Some(runtime_guard) = runtime_guard else {
        return Capture(None);
    };
    let mut guard = Guard::fleet(&generation.fleet_key, runtime_guard);
    guard.bind_generation(generation);
    Capture::start(store.diagnostic_sink(), guard, lane, question, now)
}

pub(crate) async fn acquire<'a>(
    semaphore: &'a tokio::sync::Semaphore,
    observer: Option<Observer>,
    code: Code,
    now: i64,
) -> Result<tokio::sync::SemaphorePermit<'a>, tokio::sync::AcquireError> {
    match semaphore.try_acquire() {
        Ok(permit) => Ok(permit),
        Err(tokio::sync::TryAcquireError::NoPermits) => {
            if let Some(mut ticket) = observer.as_ref().and_then(|o| o.begin(now)) {
                ticket.observation.question.reason(
                    code,
                    StageId::ExecutionAdmission,
                    Effect::Informational,
                    EvidenceKind::ControllerObservation,
                );
                ticket
                    .observation
                    .question
                    .stage(StageId::ExecutionAdmission, Evaluation::Pending);
                ticket.observation.question.outcome = Outcome::Progressing;
                ticket.publish();
            }
            let permit = semaphore.acquire().await?;
            if let Some(mut ticket) = observer.as_ref().and_then(|o| o.begin(now)) {
                ticket
                    .observation
                    .question
                    .stage(StageId::ExecutionAdmission, Evaluation::Passed);
                ticket.publish();
            }
            Ok(permit)
        }
        Err(tokio::sync::TryAcquireError::Closed) => semaphore.acquire().await,
    }
}

pub(crate) fn generation_observer(
    store: &dyn shaula_core::registry::ControlPlaneStore,
    runtime_guard: Option<&shaula_core::registry::FleetRuntimeGuard>,
    generation: &shaula_core::registry::GenerationRecord,
    lane: Lane,
    question: QuestionId,
) -> Option<Observer> {
    let mut guard = Guard::fleet(&generation.fleet_key, runtime_guard?);
    guard.bind_generation(generation);
    Observer::register(store.diagnostic_sink(), guard, lane, question)
}
