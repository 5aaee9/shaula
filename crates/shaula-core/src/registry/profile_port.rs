//! The Profile Registry driving port.

use async_trait::async_trait;

use crate::error::CoreResult;
use crate::registry::{
    Actor, AttestationPut, AuthProfilePut, AuthProfileView, ChangeView, MutationAccepted,
    MutationError, TemplateProfilePut, TemplateProfileView,
};

/// Read model of ONE immutable Template Revision (R10-05).
#[derive(Debug, Clone)]
pub struct TemplateRevisionView {
    pub profile_key: String,
    pub revision: i64,
    pub artifact_digest: String,
    pub engine_ref: String,
    pub platform: Option<String>,
    pub bindings_contract: Option<String>,
    pub state: String,
    pub bindings_present: bool,
}

/// Read model of ONE immutable attestation (R10-05). The subject is the
/// canonical typed serialization — raw output, bindings and credentials
/// can never appear here by construction.
#[derive(Debug, Clone)]
pub struct AttestationView {
    pub profile_key: String,
    pub revision: i64,
    pub subject: serde_json::Value,
    pub result: String,
    pub suite: (String, String),
    pub completed_at: i64,
    pub subject_verified: bool,
}

/// Read model of ONE immutable Auth Revision (R10-05). Credential bytes
/// are excluded by construction — metadata only.
#[derive(Debug, Clone)]
pub struct AuthRevisionView {
    pub profile_key: String,
    pub revision: i64,
    pub kind: String,
    pub app_id: Option<String>,
    pub installation_id: Option<i64>,
    pub pat_principal: Option<String>,
}

/// The Profile Registry driving port.
#[async_trait]
pub trait ProfileRegistryPort: Send + Sync {
    /// Conditional publish (R9-02, spec 0005 §3): `if_none_match` is the
    /// parsed `If-None-Match: *` create-only precondition, `if_match` the
    /// parsed strong `If-Match: "incarnation:revision"` ETag. Neither is
    /// optional at the protocol level: a PUT with neither is 428, and a
    /// stale ETag can never advance the desired head.
    async fn template_put(
        &self,
        actor: &Actor,
        key: &str,
        payload: TemplateProfilePut,
        if_none_match: bool,
        if_match: Option<(String, i64)>,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>>;

    async fn template_get(
        &self,
        actor: &Actor,
        key: &str,
    ) -> CoreResult<Result<TemplateProfileView, MutationError>>;
    async fn template_list(&self, actor: &Actor) -> CoreResult<Vec<TemplateProfileView>>;

    /// Read model of ONE immutable Template Revision (R10-05 route).
    async fn template_revision_get(
        &self,
        actor: &Actor,
        key: &str,
        revision: i64,
    ) -> CoreResult<Result<TemplateRevisionView, MutationError>>;

    /// Read model of ONE immutable attestation (R10-05 route). The stored
    /// subject is the canonical typed serialization — never secret.
    async fn attestation_get(
        &self,
        actor: &Actor,
        key: &str,
        revision: i64,
        attestation_key: &str,
    ) -> CoreResult<Result<AttestationView, MutationError>>;

    async fn template_delete(
        &self,
        actor: &Actor,
        key: &str,
        idempotency_key: Option<String>,
        if_match: Option<(String, i64)>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>>;

    async fn attestation_put(
        &self,
        actor: &Actor,
        key: &str,
        revision: i64,
        payload: AttestationPut,
    ) -> CoreResult<Result<String, MutationError>>;

    async fn auth_put(
        &self,
        actor: &Actor,
        key: &str,
        payload: AuthProfilePut,
        if_none_match: bool,
        if_match: Option<(String, i64)>,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>>;

    async fn auth_get(
        &self,
        actor: &Actor,
        key: &str,
    ) -> CoreResult<Result<AuthProfileView, MutationError>>;
    async fn auth_list(&self, actor: &Actor) -> CoreResult<Vec<AuthProfileView>>;

    /// Read model of ONE immutable Auth Revision — credential bytes
    /// excluded (R10-05 route).
    async fn auth_revision_get(
        &self,
        actor: &Actor,
        key: &str,
        revision: i64,
    ) -> CoreResult<Result<AuthRevisionView, MutationError>>;

    async fn auth_delete(
        &self,
        actor: &Actor,
        key: &str,
        idempotency_key: Option<String>,
        if_match: Option<(String, i64)>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>>;

    async fn profile_change_get(
        &self,
        actor: &Actor,
        change_id: &str,
    ) -> CoreResult<Option<ChangeView>>;
}

/// Outcome of an idempotency lookup before admission.
#[derive(Debug, Clone, PartialEq)]
pub enum IdempotencyLookup {
    /// Key is new; proceed with conditional validation.
    Miss,
    /// Same key + same request hash: replay the stored response.
    Replay(String),
    /// Same key + different request hash: reject, never silently replay.
    Conflict,
}
