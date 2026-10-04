//! Proven provider facts consumed by Auth candidate validation use cases.
use crate::{
    auth_context::RepositorySelection,
    auth_policy::{AccountKind, TargetSelector},
    error::CoreResult,
    forgejo::ForgejoTarget,
    github::GitHubTarget,
    ports::forgejo::ForgejoAuthProbe,
    secret::SecretString,
};

/// Verified installation facts for one account (non-secret).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallationProof {
    pub installation_id: i64,
    pub app_id: i64,
    pub account_id: i64,
    pub account_kind: AccountKind,
    /// Canonical login as GitHub returned it (display only).
    pub login: String,
    pub repository_selection: RepositorySelection,
    pub has_required_permission: bool,
}

/// Classified lookup outcome; the validator maps every variant onto a
/// durable Candidate outcome or a bounded retry (spec 0011 §4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallationLookup {
    Proven(InstallationProof),
    /// 404: no installation for this account/repository.
    NotFound,
    /// The installation exists but is suspended.
    Suspended,
    /// The installation answers for a DIFFERENT App or account than the
    /// declared/selector identity.
    IdentityMismatch,
    /// A required permission is missing from the installation.
    PermissionDenied,
    /// Network failure, `429`, rate-limit `403` or GitHub `5xx`: bounded
    /// retry, never a terminal rejection.
    /// Bounded retry with the deadline GitHub supplied (if any).
    Transient {
        retry_after_ms: Option<i64>,
    },
}

/// Outcome of the bounded installation-metadata probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataReachability {
    Reachable,
    NotFound,
    PermissionDenied,
    Transient { retry_after_ms: Option<i64> },
}

/// The `/app` proof of the declared numeric App identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppVerification {
    pub app_id: i64,
}

/// Classified outcome of one repository identity lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoIdentity {
    /// Durable numeric identity (repository id, owner id).
    Proven(i64, i64),
    /// 404: removed from the installation, renamed or deleted.
    Missing,
    Transient {
        retry_after_ms: Option<i64>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeOutcome {
    Proven,
    Terminal(&'static str),
    Retry { retry_after_ms: Option<i64> },
}

#[async_trait::async_trait]
pub trait GitHubValidationPort: Send + Sync {
    async fn verify_app(&self, app_id: &str, private_key: &SecretString) -> ProbeOutcome;
    async fn installation_for_selector(
        &self,
        app_id: &str,
        private_key: &SecretString,
        selector: &TargetSelector,
    ) -> InstallationLookup;
    async fn installation_metadata_reachable(
        &self,
        app_id: &str,
        private_key: &SecretString,
        installation_id: i64,
    ) -> MetadataReachability;
    async fn repository_identity(
        &self,
        app_id: &str,
        private_key: &SecretString,
        installation_id: i64,
        owner: &str,
        repository: &str,
    ) -> RepoIdentity;
    async fn probe_runner_access(
        &self,
        target: &GitHubTarget,
        app_id: &str,
        installation_id: i64,
        private_key: &SecretString,
    ) -> ProbeOutcome;
}

/// The composition root supplies concrete clients; application policy owns all
/// validation, predecessor continuity, promotion and scheduling decisions.
#[async_trait::async_trait]
pub trait AuthValidationFactory: Send + Sync {
    fn github(&self) -> CoreResult<Box<dyn GitHubValidationPort>>;
    async fn forgejo(
        &self,
        target: &ForgejoTarget,
        token: SecretString,
    ) -> CoreResult<Result<ForgejoAuthProbe, ProbeOutcome>>;
}
