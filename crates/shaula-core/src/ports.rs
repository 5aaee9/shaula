//! Caller-facing ports owned by the core. Adapters implement them; adapter
//! DTOs, SeaORM entities and wire models never cross these traits.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::auth::AuthKind;
use crate::github::GitHubTarget;
use crate::github::Label;
use crate::github::ScaleSetIdentity;
use crate::template::BindingsDigest;
use crate::template::ShaulaInputEnvelope;
use crate::template::ShaulaResultEnvelope;

/// Deterministic clock port; production impl uses the system clock.
#[async_trait]
pub trait Clock: Send + Sync {
    fn now_unix_ms(&self) -> i64;
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp_millis(self.now_unix_ms()).unwrap_or_default()
    }
}

/// Deterministic identity port; production impl uses UUIDv4.
pub trait IdGenerator: Send + Sync {
    fn new_id(&self) -> String;
}

/// Typed GitHub access failure. `401`/`403`/access-filtered `404` are access
/// failures — never absence proofs and never triggers for Create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessFailure {
    Unauthenticated,
    /// The message-queue token expired; the caller must re-establish the
    /// session (single refresh+retry mirrors the Go SDK), never treat it
    /// as credential loss.
    SessionExpired,
    PermissionDenied,
    TargetHiddenOrNotFound,
    RateLimited {
        retry_after: Option<Duration>,
    },
    Unavailable {
        summary: String,
    },
    RequestUncertain {
        summary: String,
    },
}

impl AccessFailure {
    pub fn summary(&self) -> String {
        match self {
            AccessFailure::Unauthenticated => "unauthenticated".into(),
            AccessFailure::SessionExpired => "session expired".into(),
            AccessFailure::PermissionDenied => "permission denied".into(),
            AccessFailure::TargetHiddenOrNotFound => "target hidden or not found".into(),
            AccessFailure::RateLimited { retry_after } => {
                format!("rate limited (retry_after={retry_after:?})")
            }
            AccessFailure::Unavailable { summary } => format!("unavailable: {summary}"),
            AccessFailure::RequestUncertain { summary } => format!("uncertain: {summary}"),
        }
    }
}

/// Authenticated principal context for one port instance.
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub kind: AuthKind,
}

/// A scale set as observed through authenticated reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScaleSetView {
    pub id: i64,
    pub name: String,
    pub runner_group_id: i64,
    pub runner_group_name: String,
    // R9-05: observable labels — the ownership proof must verify the set
    // is compatible BEFORE binding (adopt) and on every later tick.
    pub labels: Vec<crate::github::Label>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupOutcome {
    ExactlyOne(ScaleSetView),
    None,
    Multiple,
}

/// Lookup by stable runner name in an exact scale set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerLookup {
    ExactlyOne(RunnerRef),
    None,
    Multiple,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovalOutcome {
    Removed,
    /// Authoritative already-absent for a still-bound, readable scale set.
    AlreadyAbsent,
    /// GitHub reports the runner still executing a job; retry later.
    JobStillRunning,
}

/// Runner as observed in the GitHub inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerRef {
    pub id: i64,
    pub name: String,
    pub scale_set_id: i64,
    /// Inventory status ("online" / "offline"; unknown values are
    /// treated as not-online). Drives generation readiness (spec 0024).
    pub status: String,
}

/// Message produced by a poll. Owned idempotent facts derive from these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PollMessage {
    pub message_id: i64,
    pub statistics: StatisticsSnapshot,
    pub job_available: Vec<JobMessage>,
    pub job_assigned: Vec<JobMessage>,
    pub job_started: Vec<JobStartedMessage>,
    pub job_completed: Vec<JobCompletedMessage>,
}

/// Latest statistics; always an overwriting snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct StatisticsSnapshot {
    pub total_assigned_jobs: i64,
    pub total_registered_runners: i64,
    pub total_busy_runners: i64,
    pub total_idle_runners: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobMessage {
    pub runner_request_id: i64,
    pub job_id: String,
    #[serde(default, skip_serializing_if = "crate::jobs::JobMetadata::is_empty")]
    pub metadata: crate::jobs::JobMetadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobStartedMessage {
    pub runner_request_id: i64,
    pub job_id: String,
    pub runner_id: i64,
    pub runner_name: String,
    #[serde(default, skip_serializing_if = "crate::jobs::JobMetadata::is_empty")]
    pub metadata: crate::jobs::JobMetadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobCompletedMessage {
    pub runner_request_id: i64,
    pub job_id: String,
    pub runner_id: i64,
    pub runner_name: String,
    #[serde(default, skip_serializing_if = "crate::jobs::JobMetadata::is_empty")]
    pub metadata: crate::jobs::JobMetadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
}

/// A live message session with its queue access material.
#[derive(Clone)]
pub struct SessionHandle {
    pub session_id: String,
    pub message_queue_url: String,
    pub message_queue_access_token: String,
    pub initial_statistics: StatisticsSnapshot,
}

// Type-bound redaction: the queue token is a credential and must never
// appear through Debug formatting.
impl std::fmt::Debug for SessionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionHandle")
            .field("session_id", &self.session_id)
            .field("message_queue_url", &self.message_queue_url)
            .field("message_queue_access_token", &"[REDACTED]")
            .field("initial_statistics", &self.initial_statistics)
            .finish()
    }
}

/// Outcome of a message poll. `NoMessage` corresponds to the long-poll 202.
#[derive(Debug, Clone)]
pub enum PollOutcome {
    NoMessage,
    Message(PollMessage),
    /// Session material expired; caller must establish a fresh session.
    SessionExpired,
}

/// The GitHub Scale Set access port. Production adapter: `shaula-scaleset`.
/// Credential or adapter concrete types never cross this interface; failures
/// never trigger auth-kind or revision fallback.
#[async_trait]
pub trait GitHubAccessPort: Send + Sync {
    async fn auth_context(&self) -> AuthContext;

    /// Bounded authenticated read; resolves the runner group by name.
    async fn resolve_runner_group(&self, identity: &ScaleSetIdentity)
        -> Result<i64, AccessFailure>;

    async fn lookup_scale_set(
        &self,
        identity: &ScaleSetIdentity,
        runner_group_id: i64,
    ) -> Result<LookupOutcome, AccessFailure>;

    /// Creates the scale set. Timeout/connection reset/body loss yields
    /// `EffectOutcome::Uncertain` — the caller must reconcile by lookup and
    /// never blindly re-POST.
    async fn create_scale_set(
        &self,
        identity: &ScaleSetIdentity,
        runner_group_id: i64,
        labels: &[Label],
    ) -> Result<EffectOutcome<ScaleSetView>, AccessFailure>;

    /// Replaces only routing labels on an already-proven owned scale set.
    /// Callers must read back the same identity after an uncertain effect.
    async fn update_scale_set_labels(
        &self,
        _scale_set_id: i64,
        _labels: &[Label],
    ) -> Result<EffectOutcome<ScaleSetView>, AccessFailure> {
        Err(AccessFailure::Unavailable {
            summary: "scale set label updates are not supported by this adapter".into(),
        })
    }

    async fn establish_session(
        &self,
        scale_set_id: i64,
        owner: &str,
    ) -> Result<EffectOutcome<SessionHandle>, AccessFailure>;

    async fn delete_session(
        &self,
        scale_set_id: i64,
        session_id: &str,
    ) -> Result<(), AccessFailure>;

    /// Long poll. `X-ScaleSetMaxCapacity` is always the fleet max.
    async fn poll_messages(
        &self,
        session: &SessionHandle,
        last_message_id: i64,
        max_capacity: i64,
    ) -> Result<PollOutcome, AccessFailure>;

    /// ACK one message.
    async fn ack_message(
        &self,
        session: &SessionHandle,
        message_id: i64,
    ) -> Result<(), AccessFailure>;

    /// Acquire job request IDs after ACK.
    async fn acquire_jobs(
        &self,
        scale_set_id: i64,
        session: &SessionHandle,
        request_ids: &[i64],
    ) -> Result<EffectOutcome<Vec<i64>>, AccessFailure>;

    /// One-time JIT registration payload for a runner name.
    async fn generate_jit(
        &self,
        scale_set_id: i64,
        runner_name: &str,
    ) -> Result<EffectOutcome<JitConfig>, AccessFailure>;

    /// Bounded lookup by stable runner name in the exact scale set.
    async fn get_runner_by_name(
        &self,
        scale_set_id: i64,
        runner_name: &str,
    ) -> Result<RunnerLookup, AccessFailure>;

    /// Safe removal with `JobStillRunning` classification.
    async fn remove_runner(&self, runner_id: i64) -> Result<RemovalOutcome, AccessFailure>;

    /// Full inventory re-observation.
    async fn list_runners(&self, scale_set_id: i64) -> Result<Vec<RunnerRef>, AccessFailure>;

    /// Validates the target allowlist for the credential; used by auth
    /// candidate validation and handoff classification.
    fn allows_target(&self, target: &GitHubTarget) -> bool;

    /// Verified numeric identity of a concrete Target (spec 0011 §4.2):
    /// organization id, or repository id + owner id. Powers the
    /// durable-pin check against same-name rebuilds.
    async fn resolve_target_identity(
        &self,
        target: &GitHubTarget,
    ) -> Result<TargetIdentity, AccessFailure>;

    /// Fresh bounded route proof for THIS port's exact target + context
    /// (spec 0011 §5.3): positive evidence reused ≤60s, negative ≤15s; a
    /// refresh RE-CHECKS installation identity/suspension, account
    /// ownership and target numeric identity — a new token alone never
    /// extends a proof. New Creates/adopts/acquisitions gate on this.
    async fn ensure_route_proof(&self) -> Result<RouteProof, AccessFailure>;

    /// Original cache window for a returned denial, for display only. Reading
    /// a cached failure must not present it as a new remote access check.
    async fn route_proof_failure_window(&self, _failure: &AccessFailure) -> Option<(i64, i64)> {
        None
    }
}

/// Numeric remote identity of a Fleet Target (authenticated read).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetIdentity {
    /// `GET /orgs/{org}` id for organization Targets.
    pub organization_id: Option<i64>,
    /// `GET /repos/{owner}/{repo}` id for repository Targets.
    pub repository_id: Option<i64>,
    /// The repository owner's account id for repository Targets.
    pub repository_owner_id: Option<i64>,
}

/// Positive route evidence with an explicit freshness window (§5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteProof {
    pub checked_at_unix_ms: i64,
    pub valid_until_unix_ms: i64,
    pub installation_id: i64,
    pub account_id: i64,
    pub organization_id: Option<i64>,
    pub repository_id: Option<i64>,
    pub repository_owner_id: Option<i64>,
}

/// One-time JIT bootstrap payload. Secret-classified.
#[derive(Clone)]
pub struct JitConfig {
    pub encoded: String,
    pub runner: RunnerRef,
}

impl std::fmt::Debug for JitConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JitConfig")
            .field("runner", &self.runner)
            .field("encoded", &"REDACTED")
            .finish()
    }
}

/// Effect that may or may not have happened remotely.
#[derive(Debug, Clone)]
pub enum EffectOutcome<T> {
    Definite(T),
    /// Transport-level uncertainty: the caller must classify by lookup,
    /// never blind-retry.
    Uncertain {
        summary: String,
    },
}

impl<T> EffectOutcome<T> {
    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> EffectOutcome<U> {
        match self {
            EffectOutcome::Definite(value) => EffectOutcome::Definite(f(value)),
            EffectOutcome::Uncertain { summary } => EffectOutcome::Uncertain { summary },
        }
    }

    pub fn definite(self) -> Option<T> {
        match self {
            EffectOutcome::Definite(value) => Some(value),
            EffectOutcome::Uncertain { .. } => None,
        }
    }
}

#[path = "ports/provenance.rs"]
mod provenance;
pub use provenance::{PlanProvenance, StateLineage};

#[path = "ports/template_runtime.rs"]
pub mod template_runtime;

pub use template_runtime::{
    ApplyClaim, ApplyIntentSink, DestroyClassification, OriginalStateIdentity,
    TemplateCreateRequest, TemplateCreateResult, TemplateDestroyRequest, TemplateOutcomeError,
    TemplateRuntimePort,
};
