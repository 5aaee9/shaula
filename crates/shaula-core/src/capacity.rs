//! Fleet capacity arithmetic (spec 0001 section 9).
//!
//! `TotalAssignedJobs` is the latest overwriting snapshot, never an
//! accumulating counter. All functions here are pure and deterministic.

/// Latest Assigned Demand snapshot for one fleet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AssignedDemand {
    pub total_assigned_jobs: i64,
}

/// Capacity policy from the Fleet spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapacityPolicy {
    pub min_runners: i64,
    pub max_runners: i64,
}

impl CapacityPolicy {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.min_runners < 0 || self.max_runners < 0 {
            return Err("capacity values must be non-negative");
        }
        if self.max_runners < self.min_runners {
            return Err("max_runners must be >= min_runners");
        }
        // Bounded capacity keeps external demand snapshots from ever
        // overflowing the arithmetic (finite allowlist requirement).
        const MAX_CAPACITY: i64 = 100_000;
        if self.min_runners > MAX_CAPACITY || self.max_runners > MAX_CAPACITY {
            return Err("capacity values must be <= 100000");
        }
        Ok(())
    }
}

/// Runner generation capacity accounting per fleet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CapacityCounters {
    /// CreatePending + Creating + WaitingOnline + Idle + Busy.
    pub effective_capacity: i64,
    /// Every generation whose Destroy has not completed, including cleanup,
    /// retiring and quarantined states.
    pub resource_occupancy: i64,
}

/// `target = min(maxRunners, minRunners + TotalAssignedJobs)`
pub fn target(policy: &CapacityPolicy, demand: &AssignedDemand) -> i64 {
    // Demand is an external snapshot; saturate instead of overflowing.
    policy.max_runners.min(
        policy
            .min_runners
            .saturating_add(demand.total_assigned_jobs),
    )
}

/// How many creates the fleet may issue now.
pub fn create_count(
    policy: &CapacityPolicy,
    demand: &AssignedDemand,
    counters: &CapacityCounters,
) -> i64 {
    let t = target(policy, demand);
    (t - counters.effective_capacity)
        .max(0)
        .min((policy.max_runners - counters.resource_occupancy).max(0))
}

/// Whether scale-down may pick this candidate: observed Idle only, but a
/// stale Busy observation must not block candidate checks forever — the
/// GitHub `JobStillRunning` removal gate remains the authoritative safety.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaleDownCandidate {
    ObservedIdle,
    StaleBusyObservation,
}

pub fn scale_down_eligible(candidate: ScaleDownCandidate) -> bool {
    matches!(
        candidate,
        ScaleDownCandidate::ObservedIdle | ScaleDownCandidate::StaleBusyObservation
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(min: i64, max: i64) -> CapacityPolicy {
        CapacityPolicy {
            min_runners: min,
            max_runners: max,
        }
    }

    #[test]
    fn target_is_snapshot_not_accumulation() {
        // min 2, max 20, demand 4 → 6.
        assert_eq!(
            target(
                &policy(2, 20),
                &AssignedDemand {
                    total_assigned_jobs: 4
                }
            ),
            6
        );
        // Demand exceeding max clamps to max.
        assert_eq!(
            target(
                &policy(2, 20),
                &AssignedDemand {
                    total_assigned_jobs: 100
                }
            ),
            20
        );
        // Zero demand falls back to warm min.
        assert_eq!(target(&policy(3, 20), &AssignedDemand::default()), 3);
    }

    #[test]
    fn create_count_respects_effective_and_occupancy() {
        let p = policy(0, 10);
        // Nothing running, 3 assigned → create 3.
        assert_eq!(
            create_count(
                &p,
                &AssignedDemand {
                    total_assigned_jobs: 3
                },
                &CapacityCounters::default()
            ),
            3
        );
        // Effective at target → no create.
        assert_eq!(
            create_count(
                &p,
                &AssignedDemand {
                    total_assigned_jobs: 3
                },
                &CapacityCounters {
                    effective_capacity: 6,
                    resource_occupancy: 6
                }
            ),
            0
        );
        // Occupancy near max caps creates even if demand is high.
        assert_eq!(
            create_count(
                &p,
                &AssignedDemand {
                    total_assigned_jobs: 10
                },
                &CapacityCounters {
                    effective_capacity: 0,
                    resource_occupancy: 8
                }
            ),
            2
        );
        // Occupancy over max (quarantined) never goes negative.
        assert_eq!(
            create_count(
                &p,
                &AssignedDemand {
                    total_assigned_jobs: 10
                },
                &CapacityCounters {
                    effective_capacity: 0,
                    resource_occupancy: 12
                }
            ),
            0
        );
    }

    #[test]
    fn policy_validation() {
        assert!(policy(0, 5).validate().is_ok());
        assert!(policy(5, 5).validate().is_ok());
        assert!(policy(6, 5).validate().is_err());
        assert!(policy(-1, 5).validate().is_err());
    }

    #[test]
    fn scale_down_candidates_both_eligible() {
        // Stale busy observations must not block candidates; the GitHub
        // removal gate is authoritative.
        assert!(scale_down_eligible(ScaleDownCandidate::ObservedIdle));
        assert!(scale_down_eligible(
            ScaleDownCandidate::StaleBusyObservation
        ));
    }
}
