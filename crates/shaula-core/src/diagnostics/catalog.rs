//! Closed catalog. Messages never incorporate provider or user text.
use super::{Severity, StageId};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Code {
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
    pub fn definition(self) -> (&'static str, StageId, Severity, &'static str) {
        match self {
            Self::ObservationMissing => (
                "observation.missing",
                StageId::Authority,
                Severity::Warning,
                "Necessary evidence has not been collected",
            ),
            Self::ObservationStale => (
                "observation.stale",
                StageId::Authority,
                Severity::Warning,
                "The observation has expired",
            ),
            Self::ObservationConflict => (
                "observation.conflict",
                StageId::Authority,
                Severity::Warning,
                "Observation identity or ordering conflicts",
            ),
            Self::ObservationClockInvalid => (
                "observation.clock_invalid",
                StageId::Authority,
                Severity::Warning,
                "Observation time cannot be trusted",
            ),
            Self::ObservationUnclassifiedFailure => (
                "observation.unclassified_failure",
                StageId::Authority,
                Severity::Warning,
                "An observation failed without a safe classification",
            ),
            Self::ControlAuthContextPending => (
                "control.auth_context_pending",
                StageId::Authority,
                Severity::Info,
                "Authentication context is not ready",
            ),
            Self::ControlDependencyUnavailable => (
                "control.dependency_unavailable",
                StageId::Authority,
                Severity::Warning,
                "A required dependency is unavailable",
            ),
            Self::ControlOwnershipUnproven => (
                "control.ownership_unproven",
                StageId::Authority,
                Severity::Warning,
                "Ownership could not be proved",
            ),
            Self::ControlListenerNotReady => (
                "control.listener_not_ready",
                StageId::Demand,
                Severity::Info,
                "The listener is not ready",
            ),
            Self::ControlDemandUnavailable => (
                "control.demand_unavailable",
                StageId::Demand,
                Severity::Warning,
                "Demand is unavailable",
            ),
            Self::ControlInventoryUnavailable => (
                "control.inventory_unavailable",
                StageId::RunnerInventory,
                Severity::Warning,
                "Runner inventory is unavailable",
            ),
            Self::ControlRateLimited => (
                "control.rate_limited",
                StageId::Authority,
                Severity::Warning,
                "The backend reported a rate limit",
            ),
            Self::ControlDecommissioning => (
                "control.decommissioning",
                StageId::Authority,
                Severity::Info,
                "Fleet decommissioning has been requested",
            ),
            Self::CapacityTargetSatisfied => (
                "capacity.target_satisfied",
                StageId::Capacity,
                Severity::Info,
                "Effective capacity meets the observed target",
            ),
            Self::CapacityPolicyCeiling => (
                "capacity.policy_ceiling",
                StageId::Capacity,
                Severity::Info,
                "The capacity policy caps the target",
            ),
            Self::CapacityOccupancyLimit => (
                "capacity.occupancy_limit",
                StageId::Capacity,
                Severity::Warning,
                "Resource occupancy leaves no create headroom",
            ),
            Self::PoolMemberAtCap => (
                "pool.member_at_cap",
                StageId::Pool,
                Severity::Info,
                "A pool member reached its admission cap",
            ),
            Self::PoolNoEligibleMember => (
                "pool.no_eligible_member",
                StageId::Pool,
                Severity::Warning,
                "The actual candidate set is empty",
            ),
            Self::PoolInlineBackpressure => (
                "pool.inline_backpressure",
                StageId::Pool,
                Severity::Warning,
                "An inline pool member prevents admission",
            ),
            Self::ExecutionCreateSlotWait => (
                "execution.create_slot_wait",
                StageId::ExecutionAdmission,
                Severity::Info,
                "Waiting for a create execution slot",
            ),
            Self::ExecutionDestroySlotWait => (
                "execution.destroy_slot_wait",
                StageId::ExecutionAdmission,
                Severity::Info,
                "Waiting for a destroy execution slot",
            ),
            Self::LifecycleApplyOutcomeUnknown => (
                "lifecycle.apply_outcome_unknown",
                StageId::CreateEffect,
                Severity::Warning,
                "Apply may have started; its outcome is unknown",
            ),
            Self::LifecycleOperationFailed => (
                "lifecycle.operation_failed",
                StageId::CreateEffect,
                Severity::Error,
                "A recorded operation failed",
            ),
            Self::LifecycleBootstrapPending => (
                "lifecycle.bootstrap_pending",
                StageId::Bootstrap,
                Severity::Info,
                "Bootstrap has not completed",
            ),
            Self::LifecycleAwaitingOnline => (
                "lifecycle.awaiting_online",
                StageId::RunnerInventory,
                Severity::Info,
                "Waiting for runner inventory to confirm readiness",
            ),
            Self::LifecycleReadinessTimeout => (
                "lifecycle.readiness_timeout",
                StageId::RunnerInventory,
                Severity::Warning,
                "The controller classified a readiness timeout",
            ),
            Self::CleanupJobBusy => (
                "cleanup.job_busy",
                StageId::SafeRemoval,
                Severity::Info,
                "Ordinary cleanup is waiting for a busy job",
            ),
            Self::CleanupSafeDrainUnavailable => (
                "cleanup.safe_drain_unavailable",
                StageId::SafeRemoval,
                Severity::Warning,
                "Safe drain proof is unavailable",
            ),
            Self::CleanupRegistrationPending => (
                "cleanup.registration_pending",
                StageId::RegistrationCleanup,
                Severity::Warning,
                "Registration cleanup has not completed",
            ),
            Self::CleanupQuarantined => (
                "cleanup.quarantined",
                StageId::Completion,
                Severity::Warning,
                "The generation requires external verification",
            ),
            Self::CleanupHardLifetime => (
                "cleanup.hard_lifetime",
                StageId::Intent,
                Severity::Warning,
                "Maximum lifetime cleanup may interrupt a busy job",
            ),
            Self::CleanupNoCreateEffect => (
                "cleanup.no_create_effect",
                StageId::ResourceCleanup,
                Severity::Info,
                "The controller proved that create apply never started",
            ),
            Self::CleanupCompleted => (
                "cleanup.completed",
                StageId::Completion,
                Severity::Info,
                "The domain ledger records cleanup completion",
            ),
            Self::RolloutNoActiveRevision => (
                "rollout.no_active_revision",
                StageId::DependencyResolution,
                Severity::Warning,
                "No active dependency revision is available",
            ),
            Self::RolloutInputsIncompatible => (
                "rollout.inputs_incompatible",
                StageId::InputCompatibility,
                Severity::Warning,
                "Retained inputs do not satisfy the new revision",
            ),
            Self::RolloutBackendIncompatible => (
                "rollout.backend_incompatible",
                StageId::InputCompatibility,
                Severity::Warning,
                "The revision does not support this backend",
            ),
            Self::RolloutWaitingZeroOccupancy => (
                "rollout.waiting_zero_occupancy",
                StageId::Occupancy,
                Severity::Info,
                "Following the new revision requires zero occupancy",
            ),
            Self::RolloutCommitDeferred => (
                "rollout.commit_deferred",
                StageId::CommitFence,
                Severity::Info,
                "The rollout did not commit",
            ),
            Self::JobAssociationUnverified => (
                "job.association_unverified",
                StageId::Association,
                Severity::Info,
                "No verified generation association exists",
            ),
            Self::JobAssociationAmbiguous => (
                "job.association_ambiguous",
                StageId::Association,
                Severity::Warning,
                "The generation association is ambiguous",
            ),
            Self::JobDispatchNotObservable => (
                "job.dispatch_not_observable",
                StageId::ExecutionContext,
                Severity::Info,
                "Upstream job dispatch is not observable",
            ),
        }
    }
    pub fn as_str(self) -> &'static str {
        self.definition().0
    }
}
