//! Stable reason codes and the core error type.
//!
//! Provider text, platform status and raw diagnostics never become reason
//! values; every failure crossing a module boundary uses one of these codes.

use thiserror::Error;

/// Stable, finite reason codes shared by conditions, changes and telemetry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReasonCode {
    SpecAccepted,
    SpecInvalid,
    TemplateNotFound,
    TemplateNotActive,
    TemplateInvalid,
    TemplatePlanFailed,
    TemplateExecutionFailed,
    AuthProfileNotFound,
    AuthTargetDenied,
    CredentialMalformed,
    Unauthenticated,
    PermissionDenied,
    TargetHiddenOrNotFound,
    RateLimited,
    TargetNotAllowed,
    InstallationNotFound,
    InstallationSuspended,
    InstallationChanged,
    TargetIdentityChanged,
    TargetPolicyInUse,
    AmbiguousInstallation,
    AccessVerificationFailed,
    OwnershipProofFailed,
    OwnershipConflict,
    ScaleSetMissing,
    ScaleSetMissingWithResources,
    ScaleSetCreateUncertain,
    ScaleSetLabelsPending,
    ScaleSetLabelsUpdateUncertain,
    UnknownRemoteRunner,
    JobStillRunning,
    JitOutcomeUncertain,
    QuotaExceeded,
    OccupiedNonZero,
    StorageUnavailable,
    Internal,
}

impl ReasonCode {
    /// Canonical wire/log representation. Never contains provider text.
    pub fn as_str(self) -> &'static str {
        match self {
            ReasonCode::SpecAccepted => "SpecAccepted",
            ReasonCode::SpecInvalid => "SpecInvalid",
            ReasonCode::TemplateNotFound => "TemplateNotFound",
            ReasonCode::TemplateNotActive => "TemplateNotActive",
            ReasonCode::TemplateInvalid => "TemplateInvalid",
            ReasonCode::TemplatePlanFailed => "TemplatePlanFailed",
            ReasonCode::TemplateExecutionFailed => "TemplateExecutionFailed",
            ReasonCode::AuthProfileNotFound => "AuthProfileNotFound",
            ReasonCode::AuthTargetDenied => "AuthTargetDenied",
            ReasonCode::CredentialMalformed => "CredentialMalformed",
            ReasonCode::Unauthenticated => "Unauthenticated",
            ReasonCode::PermissionDenied => "PermissionDenied",
            ReasonCode::TargetHiddenOrNotFound => "TargetHiddenOrNotFound",
            ReasonCode::RateLimited => "RateLimited",
            ReasonCode::TargetNotAllowed => "TargetNotAllowed",
            ReasonCode::InstallationNotFound => "InstallationNotFound",
            ReasonCode::InstallationSuspended => "InstallationSuspended",
            ReasonCode::InstallationChanged => "InstallationChanged",
            ReasonCode::TargetIdentityChanged => "TargetIdentityChanged",
            ReasonCode::TargetPolicyInUse => "TargetPolicyInUse",
            ReasonCode::AmbiguousInstallation => "AmbiguousInstallation",
            ReasonCode::AccessVerificationFailed => "AccessVerificationFailed",
            ReasonCode::OwnershipProofFailed => "OwnershipProofFailed",
            ReasonCode::OwnershipConflict => "OwnershipConflict",
            ReasonCode::ScaleSetMissing => "ScaleSetMissing",
            ReasonCode::ScaleSetMissingWithResources => "ScaleSetMissingWithResources",
            ReasonCode::ScaleSetCreateUncertain => "ScaleSetCreateUncertain",
            ReasonCode::ScaleSetLabelsPending => "ScaleSetLabelsPending",
            ReasonCode::ScaleSetLabelsUpdateUncertain => "ScaleSetLabelsUpdateUncertain",
            ReasonCode::UnknownRemoteRunner => "UnknownRemoteRunner",
            ReasonCode::JobStillRunning => "JobStillRunning",
            ReasonCode::JitOutcomeUncertain => "JitOutcomeUncertain",
            ReasonCode::QuotaExceeded => "QuotaExceeded",
            ReasonCode::OccupiedNonZero => "OccupiedNonZero",
            ReasonCode::StorageUnavailable => "StorageUnavailable",
            ReasonCode::Internal => "Internal",
        }
    }
}

/// Domain error with a stable reason code; never carries secret or provider
/// diagnostic content.
#[derive(Debug, Error)]
#[error("{code:?}: {summary}")]
pub struct CoreError {
    pub code: ReasonCode,
    pub summary: String,
}

impl CoreError {
    pub fn new(code: ReasonCode, summary: impl Into<String>) -> Self {
        Self {
            code,
            summary: summary.into(),
        }
    }
}

pub type CoreResult<T> = Result<T, CoreError>;
