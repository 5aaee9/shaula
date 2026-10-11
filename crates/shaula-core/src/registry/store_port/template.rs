//! Template Profiles, their revisions and catalog sources.

use async_trait::async_trait;

use super::{MutationFacts, ProfileHead, TemplateRevisionRow};
use crate::error::CoreResult;
use crate::registry::{Actor, MutationError};

#[async_trait]
pub trait TemplateStore: Send + Sync {
    async fn template_profile_get(&self, key: &str) -> CoreResult<Option<ProfileHead>>;
    async fn template_profile_keys(
        &self,
        page: crate::registry::KeyPage,
    ) -> CoreResult<Vec<String>>;
    /// Read-only live-reference observation; never authorizes retirement.
    /// `None` means the store has no reference authority to observe.
    async fn template_references_in_use(&self, key: &str, now: i64) -> CoreResult<Option<bool>>;
    /// Current trusted default catalog entry; historical associations are independent.
    async fn template_source_get(
        &self,
        key: &str,
    ) -> CoreResult<Option<crate::registry::TemplateSource>>;
    async fn template_revision_get(
        &self,
        key: &str,
        revision: i64,
    ) -> CoreResult<Option<TemplateRevisionRow>>;
    /// Protected-memory handoff of a Template Revision's bindings JSON
    /// (schema-sensitive plaintext) plus its stored commitment; used for
    /// idempotency comparison and protected envelope assembly.
    async fn template_protected_bindings(
        &self,
        key: &str,
        revision: i64,
    ) -> CoreResult<Option<(String, String)>>;
    /// Commits one Template Candidate revision with its facts. A lost
    /// fence race (a concurrent PUT advanced the head between admission
    /// and commit) surfaces as `Err(MutationError::PreconditionFailed)`,
    /// never as an infrastructure error (R9-02).
    async fn commit_template_revision(
        &self,
        facts: MutationFacts,
        extra: (String, String, String, String),
        source_key: Option<String>,
    ) -> CoreResult<Result<(), MutationError>>;
    /// Durable template-publish NO-OP (R9-02; see commits_noop impl).
    async fn commit_template_noop(
        &self,
        key: &str,
        incarnation: &str,
        revision: i64,
        actor: &Actor,
        idempotency: Option<(String, String, String)>,
        now: i64,
    ) -> CoreResult<Result<(), MutationError>>;
}
