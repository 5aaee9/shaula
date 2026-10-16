//! Prometheus series for IaC effects and Retirement outcomes. Labels carry
//! fixed codes only (effect, result class, runtime phase, quarantine
//! reason); Generation, Fleet and job identity never become labels here.

use std::future::Future;
use std::time::Instant;

use shaula_core::ports::TemplateOutcomeError;
use shaula_observability::prometheus::{count, observe, Family, Kind};

static EFFECTS: Family = Family {
    name: "shaula_iac_operations_total",
    help: "IaC create/destroy effects by result class.",
    kind: Kind::Counter,
};
static DURATION: Family = Family {
    name: "shaula_iac_operation_duration_seconds",
    help: "Wall time of IaC create/destroy effects.",
    kind: Kind::Histogram,
};
static FAILURES: Family = Family {
    name: "shaula_iac_failures_total",
    help: "Failed IaC effects by result class and fixed runtime phase (e.g. destroy.identity).",
    kind: Kind::Counter,
};
static DESTROY_ATTEMPTS: Family = Family {
    name: "shaula_destroy_attempts_total",
    help: "Runner Resource destroy attempts; retry follows an earlier attempt.",
    kind: Kind::Counter,
};
static QUARANTINES: Family = Family {
    name: "shaula_destroy_quarantines_total",
    help: "Retirements quarantined because a safe destroy could not be proven.",
    kind: Kind::Counter,
};

#[derive(Debug, Clone, Copy)]
pub(crate) enum Effect {
    Create,
    Destroy,
}

impl Effect {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Destroy => "destroy",
        }
    }
}

/// Runs one IaC effect and records its result class and duration.
pub(crate) async fn timed<T>(
    effect: Effect,
    run: impl Future<Output = Result<T, TemplateOutcomeError>>,
) -> Result<T, TemplateOutcomeError> {
    let started = Instant::now();
    let result = run.await;
    let seconds = started.elapsed().as_secs_f64();
    let (class, phase) = match &result {
        Ok(_) => ("ok", None),
        Err(TemplateOutcomeError::PlanFailed { phase }) => ("plan_failed", Some(phase)),
        Err(TemplateOutcomeError::ExecutionFailed { phase }) => ("execution_failed", Some(phase)),
        Err(TemplateOutcomeError::StateUnavailable { phase }) => ("state_unavailable", Some(phase)),
    };
    let labels = [("operation", effect.as_str()), ("result", class)];
    count(&EFFECTS, &labels, 1);
    observe(&DURATION, &labels, seconds);
    if let Some(phase) = phase {
        count(
            &FAILURES,
            &[
                ("operation", effect.as_str()),
                ("result", class),
                ("phase", phase.as_str()),
            ],
            1,
        );
    }
    result
}

/// One Runner Resource destroy attempt; `retry` when one already ran.
pub(crate) fn destroy_attempt(retry: bool) {
    let attempt = if retry { "retry" } else { "first" };
    count(&DESTROY_ATTEMPTS, &[("attempt", attempt)], 1);
}

/// One quarantined Retirement, by the fixed class of its reason text.
pub(crate) fn quarantine(reason: &str, foreign_resource: &str) {
    let class = if reason == foreign_resource {
        "foreign_resource"
    } else if reason.contains("registration cannot be proven removed") {
        "registration_unproven"
    } else if reason.ends_with("for destroy") {
        "destroy_proof_missing"
    } else {
        "other"
    };
    count(&QUARANTINES, &[("reason", class)], 1);
}
