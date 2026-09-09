//! Registry ports and read models: the Fleet/Profile HTTP adapter calls
//! these; the daemon implements them. No HTTP, SeaORM or wire types cross
//! here.

use async_trait::async_trait;

use crate::auth::AuthKind;
use crate::error::CoreResult;
use crate::fleet::{FleetSpec, TemplateProfileRefDto};

/// Authenticated principal and effective grants supplied by the HTTP adapter.
/// `name` is a versioned stable identity, not a mutable display name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Actor {
    pub name: String,
    pub scopes: Vec<Scope>,
}

impl Actor {
    pub fn has(&self, scope: Scope) -> bool {
        self.scopes.contains(&scope)
    }
}

/// Independent management capabilities (spec 0005 §8). High-trust
/// capabilities are separately grantable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    FleetRead,
    FleetWrite,
    FleetRetire,
    TemplateRead,
    TemplatePublish,
    TemplateAttest,
    TemplateRetire,
    AuthRead,
    AuthWrite,
    AuthRetire,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::FleetRead => "fleet.read",
            Scope::FleetWrite => "fleet.write",
            Scope::FleetRetire => "fleet.retire",
            Scope::TemplateRead => "template.read",
            Scope::TemplatePublish => "template.publish",
            Scope::TemplateAttest => "template.attest",
            Scope::TemplateRetire => "template.retire",
            Scope::AuthRead => "auth.read",
            Scope::AuthWrite => "auth.write",
            Scope::AuthRetire => "auth.retire",
        }
    }
}

/// Read model of the canonical desired Fleet representation.
#[derive(Debug, Clone, PartialEq)]
pub struct FleetResource {
    pub key: String,
    pub spec: FleetSpec,
    pub incarnation: String,
    pub revision: i64,
    /// Resolved admission facts; exact pin frozen at acceptance.
    pub resolved_template: Option<(String, i64, String, String)>,
    pub resolved_auth: (String, i64),
    pub created_at: i64,
    pub updated_at: i64,
}

/// Read model of the runtime Fleet status.
#[derive(Debug, Clone, PartialEq)]
pub struct FleetStatus {
    pub fleet_key: String,
    pub desired_revision: i64,
    pub observed_revision: i64,
    pub phase: String,
    pub dependencies: DependencySummary,
    pub conditions: Vec<Condition>,
    pub capacity: CapacitySummary,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DependencySummary {
    pub template: Option<(String, i64, String, String, String)>,
    /// Full desired/observed auth tuples plus handoff state.
    pub auth: Option<AuthRolloutSummary>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuthRolloutSummary {
    pub desired: (String, i64),
    pub observed: Option<(String, i64)>,
    pub handoff_state: String,
    /// Exact Resolved Auth Context rollout (spec 0011 §6): the desired
    /// and observed context refs with the durable state/reason. Missing
    /// context in historical data never authorizes execution.
    pub context: Option<AuthContextSummary>,
}

/// Non-secret context rollout summary for the fleet status surface.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthContextSummary {
    pub desired: Option<(String, i64)>,
    pub observed: Option<(String, i64)>,
    /// Pending / Observed / Blocked.
    pub state: String,
    pub reason: Option<String>,
    /// Non-secret route metadata: account/login, installation id and the
    /// target address (parsed from the context payload).
    pub desired_route: Option<serde_json::Value>,
    pub observed_route: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Condition {
    pub condition_type: &'static str,
    pub status: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CapacitySummary {
    pub assigned_demand: i64,
    pub target: i64,
    pub effective: i64,
    pub occupancy: i64,
}

/// Asynchronous Change record returned to clients.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChangeView {
    pub id: String,
    pub resource_kind: String,
    pub resource_key: String,
    pub revision: i64,
    pub kind: String,
    pub state: String,
    pub reason: Option<String>,
}

/// Outcome of an effective mutation: `202` semantics with the created facts.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MutationAccepted {
    pub etag: String,
    pub change: ChangeView,
    /// True when the mutation was an identical no-op re-assertion; HTTP
    /// maps this to a replayable 200 instead of 202.
    #[serde(default)]
    pub no_op: bool,
}

/// Non-secret Template Profile summary.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateProfileView {
    pub key: String,
    pub incarnation: String,
    pub desired_revision: i64,
    pub active_revision: Option<i64>,
    pub status: String,
    /// Derived only from the admitted artifact manifest.
    pub platform: Option<String>,
    pub bindings_contract: Option<String>,
    /// Bounded presence metadata for sensitive bindings.
    pub bindings_present: bool,
}

/// Submission payload for a Template Profile Candidate revision.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateProfilePut {
    pub artifact_digest: String,
    pub engine_ref: String,
    pub bindings: serde_json::Value,
    pub fleet_input_policy: serde_json::Value,
}

/// Submission payload for a GitHub Auth Candidate revision. Secret bytes
/// are carried as [`crate::secret::SecretString`] and never logged.
/// Only explicit schema version 2 and GitHub App policy are supported.
#[derive(Clone)]
pub struct AuthProfilePut {
    pub kind: AuthKind,
    pub app_id: Option<String>,
    pub secret: crate::secret::SecretString,
    pub schema_version: Option<i64>,
    pub target_policy: Option<Vec<crate::auth_policy::TargetSelector>>,
}

impl std::fmt::Debug for AuthProfilePut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthProfilePut")
            .field("kind", &self.kind)
            .field("credential_present", &true)
            .finish_non_exhaustive()
    }
}

/// Immutable conformance attestation submission.
#[derive(Debug, Clone, PartialEq)]
pub struct AttestationPut {
    pub attestation_key: String,
    pub subject: AttestationSubject,
    pub result: String,
    pub evidence_digest: Option<String>,
    pub suite: (String, String),
    pub completed_at: i64,
}

/// Precondition outcomes used by the HTTP adapter to map status codes.
#[derive(Debug, Clone, PartialEq)]
pub enum MutationError {
    /// 428
    PreconditionRequired,
    /// 412 with current revision metadata
    PreconditionFailed { current: (String, i64) },
    /// 409 immutable identity change
    IdentityConflict,
    /// 409 idempotency key reuse with different content
    IdempotencyConflict,
    /// 422 inadmissible spec
    Unprocessable {
        reason: crate::error::ReasonCode,
        summary: String,
    },
    /// 404
    NotFound,
    /// 410 terminal tombstone
    Gone { tombstone: String },
    /// 409 referenced resources block retirement; stays visibly blocked
    RetirementBlocked { reason: String },
    /// 429 admission/backlog limit
    TooManyRequests { retry_after_secs: u64 },
}

pub type MutationResult<T> = Result<T, MutationError>;

/// The Fleet Registry driving port.
#[async_trait]
pub trait FleetRegistryPort: Send + Sync {
    async fn fleet_put(
        &self,
        actor: &Actor,
        key: &str,
        spec: FleetSpec,
        if_none_match: bool,
        if_match: Option<(String, i64)>,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>>;

    async fn fleet_get(
        &self,
        actor: &Actor,
        key: &str,
    ) -> CoreResult<Result<FleetResource, MutationError>>;

    async fn fleet_status_get(
        &self,
        actor: &Actor,
        key: &str,
    ) -> CoreResult<Result<FleetStatus, MutationError>>;

    async fn fleet_list(&self, actor: &Actor) -> CoreResult<Vec<(String, i64, String)>>;

    async fn fleet_delete(
        &self,
        actor: &Actor,
        key: &str,
        if_match: Option<(String, i64)>,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>>;

    async fn fleet_change_get(
        &self,
        actor: &Actor,
        change_id: &str,
    ) -> CoreResult<Option<ChangeView>>;

    /// Resolves a bare or exact template reference against the current
    /// active revision; part of admission.
    async fn resolve_template_ref(
        &self,
        reference: &TemplateProfileRefDto,
    ) -> CoreResult<Option<(String, i64, String, String)>>;
}

pub mod attestation_subject;
pub use attestation_subject::{
    AttestationEngineSubject, AttestationProviderSubject, AttestationSubject,
    AttestationSuiteSubject,
};

pub mod input_contract;
#[path = "registry/template_library.rs"]
pub mod template_library;
pub use template_library::{TemplateSource, TemplateVariable, TemplateVariables};
#[path = "registry/profile_port.rs"]
pub mod profile_port;
pub use input_contract::{
    InputContractProjection, InputContractReadError, InputField, InputOption, TemplateInputContract,
};
pub use profile_port::{
    AttestationView, AuthRevisionView, IdempotencyLookup, ProfileRegistryPort, TemplateRevisionView,
};

/// Liveness/readiness source.
#[async_trait]
pub trait HealthPort: Send + Sync {
    async fn live(&self) -> bool;
    /// True only when the control plane can authenticate calls, commit
    /// durably and schedule safely.
    async fn ready(&self) -> bool;
}

pub mod attestation_port;
pub mod auth_port;
pub mod lifecycle_port;
pub mod store_port;
pub use attestation_port::{
    attestation_record_id, AttestationCommit, AttestationRecord, AttestationReplayRow,
};
pub use auth_port::{
    auth_context_pins_agree, auth_dependent_set_fingerprint, AuthBindingHealth, AuthCheckedFleet,
    AuthDependentTarget, AuthExecutionStore, AuthHandoffExpectation, AuthHandoffRow,
    AuthIdentityProof, AuthLiveFleet, AuthProfileView, AuthPromotion, AuthPromotionOutcome,
    AuthRepoProof, AuthRevisionRow, AuthRevisionState, AuthRouteObservation,
    AuthValidationSnapshot, FleetAuthContextRow, FleetContextAck,
};
pub use lifecycle_port::{FleetHeadGuard, LifecycleStore};
pub use lifecycle_port::{GenerationRecord, OperationRow, ScaleSetRow};
pub use store_port::ControlPlaneStore;
pub use store_port::MutationFacts;
pub use store_port::{FleetHead, FleetRevisionRow, ProfileHead, TemplateRevisionRow};

mod write_records;
pub use write_records::{
    AuditAppend, IdempotencyInsert, JobObservationInsert, OperationInsert, ProfileChangeInsert,
};
