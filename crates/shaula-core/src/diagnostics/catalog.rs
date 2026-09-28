//! Closed catalog. Messages never incorporate provider or user text.

use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Code {
    #[serde(rename = "worker.migration_required")]
    WorkerMigrationRequired,
    #[serde(rename = "worker.launch_unresolved")]
    WorkerLaunchUnresolved,
    #[serde(rename = "worker.fencing_unknown")]
    WorkerFencingUnknown,
    #[serde(rename = "worker.state_unavailable")]
    WorkerStateUnavailable,
    #[serde(rename = "worker.cleanup_only")]
    WorkerCleanupOnly,
    #[serde(rename = "worker.receipt_committed")]
    WorkerReceiptCommitted,
    #[serde(rename = "observation.missing")]
    ObservationMissing,
    #[serde(rename = "observation.stale")]
    ObservationStale,
    #[serde(rename = "observation.conflict")]
    ObservationConflict,
    #[serde(rename = "observation.clock_invalid")]
    ObservationClockInvalid,
    #[serde(rename = "observation.unclassified_failure")]
    ObservationUnclassifiedFailure,
    #[serde(rename = "control.auth_context_pending")]
    ControlAuthContextPending,
    #[serde(rename = "control.dependency_unavailable")]
    ControlDependencyUnavailable,
    #[serde(rename = "control.ownership_unproven")]
    ControlOwnershipUnproven,
    #[serde(rename = "control.listener_not_ready")]
    ControlListenerNotReady,
    #[serde(rename = "control.demand_unavailable")]
    ControlDemandUnavailable,
    #[serde(rename = "control.inventory_unavailable")]
    ControlInventoryUnavailable,
    #[serde(rename = "control.rate_limited")]
    ControlRateLimited,
    #[serde(rename = "control.decommissioning")]
    ControlDecommissioning,
    #[serde(rename = "capacity.target_satisfied")]
    CapacityTargetSatisfied,
    #[serde(rename = "capacity.policy_ceiling")]
    CapacityPolicyCeiling,
    #[serde(rename = "capacity.occupancy_limit")]
    CapacityOccupancyLimit,
    #[serde(rename = "pool.member_at_cap")]
    PoolMemberAtCap,
    #[serde(rename = "pool.no_eligible_member")]
    PoolNoEligibleMember,
    #[serde(rename = "pool.inline_backpressure")]
    PoolInlineBackpressure,
    #[serde(rename = "execution.create_slot_wait")]
    ExecutionCreateSlotWait,
    #[serde(rename = "execution.destroy_slot_wait")]
    ExecutionDestroySlotWait,
    #[serde(rename = "lifecycle.apply_outcome_unknown")]
    LifecycleApplyOutcomeUnknown,
    #[serde(rename = "lifecycle.operation_failed")]
    LifecycleOperationFailed,
    #[serde(rename = "lifecycle.bootstrap_pending")]
    LifecycleBootstrapPending,
    #[serde(rename = "lifecycle.awaiting_online")]
    LifecycleAwaitingOnline,
    #[serde(rename = "lifecycle.readiness_timeout")]
    LifecycleReadinessTimeout,
    #[serde(rename = "cleanup.job_busy")]
    CleanupJobBusy,
    #[serde(rename = "cleanup.safe_drain_unavailable")]
    CleanupSafeDrainUnavailable,
    #[serde(rename = "cleanup.registration_pending")]
    CleanupRegistrationPending,
    #[serde(rename = "cleanup.quarantined")]
    CleanupQuarantined,
    #[serde(rename = "cleanup.hard_lifetime")]
    CleanupHardLifetime,
    #[serde(rename = "cleanup.no_create_effect")]
    CleanupNoCreateEffect,
    #[serde(rename = "cleanup.completed")]
    CleanupCompleted,
    #[serde(rename = "rollout.no_active_revision")]
    RolloutNoActiveRevision,
    #[serde(rename = "rollout.inputs_incompatible")]
    RolloutInputsIncompatible,
    #[serde(rename = "rollout.backend_incompatible")]
    RolloutBackendIncompatible,
    #[serde(rename = "rollout.waiting_zero_occupancy")]
    RolloutWaitingZeroOccupancy,
    #[serde(rename = "rollout.commit_deferred")]
    RolloutCommitDeferred,
    #[serde(rename = "job.association_unverified")]
    JobAssociationUnverified,
    #[serde(rename = "job.association_ambiguous")]
    JobAssociationAmbiguous,
    #[serde(rename = "job.dispatch_not_observable")]
    JobDispatchNotObservable,
}
impl Code {
    pub const ALL: &[Self] = &[
        Self::WorkerMigrationRequired,
        Self::WorkerLaunchUnresolved,
        Self::WorkerFencingUnknown,
        Self::WorkerStateUnavailable,
        Self::WorkerCleanupOnly,
        Self::WorkerReceiptCommitted,
        Self::ObservationMissing,
        Self::ObservationStale,
        Self::ObservationConflict,
        Self::ObservationClockInvalid,
        Self::ObservationUnclassifiedFailure,
        Self::ControlAuthContextPending,
        Self::ControlDependencyUnavailable,
        Self::ControlOwnershipUnproven,
        Self::ControlListenerNotReady,
        Self::ControlDemandUnavailable,
        Self::ControlInventoryUnavailable,
        Self::ControlRateLimited,
        Self::ControlDecommissioning,
        Self::CapacityTargetSatisfied,
        Self::CapacityPolicyCeiling,
        Self::CapacityOccupancyLimit,
        Self::PoolMemberAtCap,
        Self::PoolNoEligibleMember,
        Self::PoolInlineBackpressure,
        Self::ExecutionCreateSlotWait,
        Self::ExecutionDestroySlotWait,
        Self::LifecycleApplyOutcomeUnknown,
        Self::LifecycleOperationFailed,
        Self::LifecycleBootstrapPending,
        Self::LifecycleAwaitingOnline,
        Self::LifecycleReadinessTimeout,
        Self::CleanupJobBusy,
        Self::CleanupSafeDrainUnavailable,
        Self::CleanupRegistrationPending,
        Self::CleanupQuarantined,
        Self::CleanupHardLifetime,
        Self::CleanupNoCreateEffect,
        Self::CleanupCompleted,
        Self::RolloutNoActiveRevision,
        Self::RolloutInputsIncompatible,
        Self::RolloutBackendIncompatible,
        Self::RolloutWaitingZeroOccupancy,
        Self::RolloutCommitDeferred,
        Self::JobAssociationUnverified,
        Self::JobAssociationAmbiguous,
        Self::JobDispatchNotObservable,
    ];
}
