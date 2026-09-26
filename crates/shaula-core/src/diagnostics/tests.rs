use super::*;
use crate::capacity::{CapacityCounters, CapacityPolicy};

#[test]
fn exact_capacity_exposes_occupancy_without_relabeling_demand_as_zero() {
    let policy = CapacityPolicy {
        min_runners: 2,
        max_runners: 10,
    };
    let counters = CapacityCounters {
        effective_capacity: 4,
        resource_occupancy: 10,
    };
    let c = capacity(
        policy,
        Some(5),
        counters,
        DemandKind::GithubTotalAssignedJobs,
    );
    assert_eq!(c.target.as_deref(), Some("7"));
    assert_eq!(c.arithmetic_create_allowance.as_deref(), Some("0"));
    assert_eq!(c.demand.as_deref(), Some("5"));
    assert_eq!(c.actually_admitted, None);
    let absent = capacity(policy, None, counters, DemandKind::ForgejoWaitingJobs);
    assert_eq!(absent.demand, None);
    assert_eq!(absent.target, None);
    let zero = capacity(policy, Some(0), counters, DemandKind::ForgejoWaitingJobs);
    assert_eq!(zero.demand.as_deref(), Some("0"));
    let large = capacity(
        policy,
        Some(i64::MAX),
        counters,
        DemandKind::GithubTotalAssignedJobs,
    );
    assert_eq!(large.demand.as_deref(), Some("9223372036854775807"));
    assert_eq!(large.target.as_deref(), Some("10"));
}

#[test]
fn catalog_is_closed_unique_and_safe_and_unknown_client_values_are_conservative() {
    let codes: std::collections::BTreeSet<_> = Code::ALL.iter().map(|c| c.as_str()).collect();
    assert_eq!(codes.len(), 41);
    for code in Code::ALL {
        assert!(code
            .as_str()
            .bytes()
            .all(|v| v.is_ascii_lowercase() || v == b'.' || v == b'_'));
        assert!(!code.definition().3.contains('<'));
        assert!(!code.definition().3.contains("${"));
    }
    assert_eq!(
        serde_json::from_str::<Outcome>("\"future_success\"").ok(),
        Some(Outcome::Unknown)
    );
    assert_eq!(
        serde_json::from_str::<Freshness>("\"future_fresh\"").ok(),
        Some(Freshness::Unknown)
    );
    let q = DiagnosticQuestion::unavailable(QuestionId::Readiness);
    assert_eq!(q.outcome, Outcome::Unknown);
    assert!(q
        .stages
        .iter()
        .all(|s| s.evaluation == Evaluation::NotEvaluated));
}

#[test]
fn pending_and_uncertain_stages_do_not_claim_they_were_never_evaluated() {
    let mut q = DiagnosticQuestion::unavailable(QuestionId::Readiness);
    q.basis.freshness = Freshness::Fresh;
    q.reason(
        Code::LifecycleApplyOutcomeUnknown,
        StageId::CreateEffect,
        Effect::Informational,
        EvidenceKind::LedgerFact,
    );
    q.reason(
        Code::LifecycleAwaitingOnline,
        StageId::RunnerInventory,
        Effect::Informational,
        EvidenceKind::BackendObservation,
    );
    assert_eq!(q.stages[0].evaluation, Evaluation::Unknown);
    assert_eq!(q.stages[1].evaluation, Evaluation::NotEvaluated);
    assert_eq!(q.stages[2].evaluation, Evaluation::Pending);
    assert_eq!(q.outcome, Outcome::Unknown);
    assert!(q.primary_reason_id.is_none());
}
