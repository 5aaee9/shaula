//! Registry ports and read models: the Fleet/Profile HTTP adapter calls
//! these; the daemon implements them. No HTTP, SeaORM or wire types cross
//! here.

use async_trait::async_trait;

use crate::auth::AuthKind;
use crate::error::CoreResult;
use crate::fleet::FleetSpec;

mod identity;
pub use identity::{Actor, AuthenticationContext, Scope};

/// Read model of the canonical desired Fleet representation.
#[derive(Debug, Clone, PartialEq)]
pub struct FleetResource {
    pub key: String,
    pub spec: FleetSpec,
    pub incarnation: String,
    pub revision: i64,
    /// Resolved admission facts; exact pin frozen at acceptance.
    pub resolved_template: Option<(String, i64, String, String)>,
    pub resolved_template_pool: Vec<crate::template_pool::ResolvedTemplatePoolMember>,
    /// Shared-pool routing context frozen at admission (spec 0037 §4).
    pub resolved_template_pool_ref: Option<crate::template_pool::FleetPoolRef>,
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

mod template_profile;
pub use template_profile::{TemplateProfilePut, TemplateProfileUpdate, TemplateProfileView};

/// Submission payload for a provider Auth Candidate. Secret bytes are
/// carried as [`crate::secret::SecretString`] and never logged. GitHub App
/// publications use schema version 2 and policy; Forgejo token publications
/// use schema version 1 and a typed scoped target.
#[derive(Clone)]
pub struct AuthProfilePut {
    pub kind: AuthKind,
    pub app_id: Option<String>,
    pub secret: crate::secret::SecretString,
    pub schema_version: Option<i64>,
    pub target_policy: Option<Vec<crate::auth_policy::TargetSelector>>,
    /// Provider-specific Forgejo target. GitHub publications leave this
    /// absent; keeping it typed prevents Forgejo routing data from being
    /// smuggled through GitHub policy fields.
    pub forgejo_target: Option<crate::forgejo::ForgejoTarget>,
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

mod mutation_error;
pub use mutation_error::MutationError;

pub type MutationResult<T> = Result<T, MutationError>;

/// The Fleet Registry driving port.
#[async_trait]
pub trait FleetRegistryPort: Send + Sync {
    async fn diagnostics(
        &self,
        _actor: &Actor,
        _kind: crate::diagnostics::SubjectKind,
        _key: &str,
    ) -> crate::diagnostics::DiagnosticsResult {
        Err(crate::diagnostics::DiagnosticsReadError::Unavailable)
    }

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

    /// Operator finalization of a Quarantined generation (spec 0028):
    /// ledger-only `Quarantined -> Destroyed` after the operator verified
    /// out-of-band that the external resources no longer exist. `reason`
    /// is the operator's bounded evidence statement persisted to audit.
    async fn generation_finalize(
        &self,
        actor: &Actor,
        generation_id: &str,
        reason: &str,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>>;
}

/// Read model of one shared TemplatePool resource (spec 0037).
#[derive(Debug, Clone)]
pub struct TemplatePoolResource {
    pub key: String,
    pub spec: crate::template_pool::TemplatePoolSpec,
    pub incarnation: String,
    pub revision: i64,
    /// Member rows resolved at the committed revision (exact Active pins).
    pub resolved_members: Vec<crate::template_pool::ResolvedTemplatePoolMember>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Shared TemplatePool registry port (spec 0037). Pools ride the template
/// permission family: publish replaces, read lists, retire tombstones —
/// the same scopes the template-profile endpoints already enforce.
#[async_trait]
pub trait TemplatePoolRegistryPort: Send + Sync {
    async fn template_pool_put(
        &self,
        actor: &Actor,
        key: &str,
        spec: crate::template_pool::TemplatePoolSpec,
        if_none_match: bool,
        if_match: Option<(String, i64)>,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>>;

    async fn template_pool_get(
        &self,
        actor: &Actor,
        key: &str,
    ) -> CoreResult<Result<TemplatePoolResource, MutationError>>;

    async fn template_pool_list(&self, actor: &Actor) -> CoreResult<Vec<(String, i64, String)>>;

    async fn template_pool_delete(
        &self,
        actor: &Actor,
        key: &str,
        if_match: Option<(String, i64)>,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>>;

    async fn template_pool_change_get(
        &self,
        actor: &Actor,
        change_id: &str,
    ) -> CoreResult<Option<ChangeView>>;
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
#[path = "registry/bindings_projection.rs"]
pub mod bindings_projection;
pub use bindings_projection::BindingsSchema;
#[path = "registry/profile_port.rs"]
pub mod profile_port;
pub use input_contract::{
    InputContractProjection, InputContractReadError, InputField, InputOption, TemplateInputContract,
};
pub use profile_port::{
    AttestationView, AuthPolicyUpdate, AuthRevisionView, IdempotencyLookup, ProfileRegistryPort,
    TemplateRevisionView,
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
mod mutation_facts;
pub mod store_port;
mod store_rows;
pub use attestation_port::{
    attestation_record_id, AttestationCommit, AttestationRecord, AttestationReplayRow,
};
pub use auth_port::{
    auth_context_pins_agree, auth_dependent_set_fingerprint, AuthBindingHealth, AuthCheckedFleet,
    AuthDependentTarget, AuthExecutionStore, AuthHandoffExpectation, AuthHandoffRow,
    AuthIdentityProof, AuthLiveFleet, AuthProfileView, AuthPromotion, AuthPromotionOutcome,
    AuthRepoProof, AuthRevisionRow, AuthRevisionState, AuthRouteObservation,
    AuthValidationSnapshot, FleetAuthContextRow, FleetContextAck, ForgejoAuthState,
};
pub use lifecycle_port::{FleetHeadGuard, LifecycleStore};
pub use lifecycle_port::{GenerationRecord, OperationRow, ScaleSetRow};
mod runtime_port;
pub use runtime_port::{
    FleetObservation, FleetObservationPhase, FleetRuntimeGuard, PersistedSession, SessionInstall,
};
pub use store_port::ControlPlaneStore;
pub use store_port::MutationFacts;
pub use store_port::{FleetHead, FleetRevisionRow, ProfileHead, TemplateRevisionRow};

mod write_records;
pub use write_records::{
    AuditAppend, IdempotencyInsert, JobObservationInsert, OperationInsert, ProfileChangeInsert,
};

pub mod listener_message_port;
pub use listener_message_port::{IngestedMessage, ListenerMessageStore, SessionEffectContext};
