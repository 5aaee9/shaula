//! Versioned, read-only explanations. All counters are exact decimal strings.
use serde::{Deserialize, Serialize};

macro_rules! wire_enum {
    ($name:ident { $($variant:ident),+ }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+, #[serde(other)] Unknown }
    };
}
wire_enum!(SubjectKind {
    Fleet,
    Generation,
    Job
});
wire_enum!(QuestionId {
    ScaleUp,
    Readiness,
    Cleanup,
    Rollout,
    JobDispatch
});
wire_enum!(Outcome {
    Satisfied,
    Progressing,
    Blocked,
    NotApplicable
});
wire_enum!(Coverage {
    Complete,
    Partial,
    None
});
wire_enum!(BasisKind {
    RecordedDecision,
    LedgerProjection,
    Unavailable
});
wire_enum!(Freshness {
    Fresh,
    Stale,
    Missing,
    NotApplicable
});
wire_enum!(Evaluation {
    Passed,
    Blocked,
    Pending,
    NotEvaluated,
    NotApplicable
});
wire_enum!(Severity {
    Info,
    Warning,
    Error
});
wire_enum!(Effect {
    Blocking,
    Informational
});
wire_enum!(EvidenceKind {
    LedgerFact,
    ControllerObservation,
    BackendObservation,
    DerivedCalculation
});
wire_enum!(DemandKind {
    GithubTotalAssignedJobs,
    ForgejoWaitingJobs
});
wire_enum!(StageId {
    Authority,
    Demand,
    Capacity,
    Pool,
    ExecutionAdmission,
    DependencyResolution,
    InputCompatibility,
    Occupancy,
    CommitFence,
    CreateEffect,
    Bootstrap,
    RunnerInventory,
    Intent,
    SafeRemoval,
    ResourceCleanup,
    RegistrationCleanup,
    Completion,
    JobObservation,
    Association,
    ExecutionContext
});
wire_enum!(CompletionSource {
    ProviderCleanup,
    NeverStarted,
    OperatorAttested
});
wire_enum!(SuggestionId {
    ViewGenerations,
    ViewGeneration,
    ViewInvocations,
    ReviewAuthDependency,
    ReviewTemplateInputs,
    WaitForReconciliation,
    ReviewExternalResources
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticSubject {
    pub kind: SubjectKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub fleet_incarnation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticReportV1 {
    pub schema_version: u32,
    pub subject: DiagnosticSubject,
    pub generated_at: String,
    pub questions: Vec<DiagnosticQuestion>,
    pub related: Vec<DiagnosticSubject>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticQuestion {
    pub question: QuestionId,
    pub outcome: Outcome,
    pub coverage: Coverage,
    pub basis: DiagnosticBasis,
    pub stages: Vec<DiagnosticStage>,
    pub primary_reason_id: Option<String>,
    pub reasons: Vec<DiagnosticReason>,
    pub evidence: Vec<DiagnosticEvidence>,
    pub suggestions: Vec<DiagnosticSuggestion>,
    pub capacity: Option<DiagnosticCapacity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool: Option<DiagnosticPool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollout: Option<DiagnosticRollout>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleanup_mode: Option<CleanupMode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticBasis {
    pub kind: BasisKind,
    pub observation_id: Option<String>,
    pub observed_at: Option<String>,
    pub valid_until: Option<String>,
    pub freshness: Freshness,
    pub subject_revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticStage {
    pub id: StageId,
    pub evaluation: Evaluation,
    pub reason_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticReason {
    pub id: String,
    /// Keep unknown literal codes for forward-compatible clients.
    pub code: String,
    pub stage: StageId,
    pub severity: Severity,
    pub effect: Effect,
    pub parameters: DiagnosticParameters,
    pub evidence_ids: Vec<String>,
    pub first_observed_at: Option<String>,
    pub last_observed_at: Option<String>,
}

/// No arbitrary metadata, error messages, input values or executable text.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticParameters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_source: Option<CompletionSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_attempt_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticEvidence {
    pub id: String,
    pub kind: EvidenceKind,
    pub observed_at: Option<String>,
    pub freshness: Freshness,
    pub valid_until: Option<String>,
    /// Finite producer predicate, never user or provider text.
    pub predicate: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticSuggestion {
    pub suggestion_id: SuggestionId,
    pub reference: Option<DiagnosticSubject>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticCapacity {
    pub demand_kind: DemandKind,
    pub min: String,
    pub max: String,
    pub demand: Option<String>,
    pub target: Option<String>,
    pub effective: String,
    pub occupancy: String,
    pub deficit: Option<String>,
    pub occupancy_headroom: String,
    pub arithmetic_create_allowance: Option<String>,
    pub actually_admitted: Option<String>,
}

wire_enum!(CleanupMode {
    Ordinary,
    HardLifetime
});
wire_enum!(RoutingMode {
    Single,
    Inline,
    Shared
});
wire_enum!(OccupancyScope {
    Fleet,
    PoolRevision
});
wire_enum!(SelectionOutcome {
    NotEvaluated,
    NoEligibleMember,
    Backpressure,
    Admitted
});
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticPool {
    pub mode: RoutingMode,
    pub scope: OccupancyScope,
    pub pool_revision: Option<String>,
    pub members: Vec<DiagnosticPoolMember>,
    pub selected_member: Option<String>,
    pub selection: SelectionOutcome,
    pub truncated: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticPoolMember {
    pub key: String,
    pub weight: String,
    pub cap: Option<String>,
    pub occupancy: String,
    pub excluded_at_cap: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticRollout {
    pub desired_revision: String,
    pub observed_revision: String,
    pub previous_pin: Option<DiagnosticPin>,
    pub candidate_pin: Option<DiagnosticPin>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticPin {
    pub key: String,
    pub revision: String,
}
