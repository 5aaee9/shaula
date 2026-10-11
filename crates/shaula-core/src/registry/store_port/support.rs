//! Cross-resource admission support: idempotency replay and diagnostics.

use async_trait::async_trait;

use crate::error::CoreResult;
use crate::registry::{Actor, IdempotencyLookup};

#[async_trait]
pub trait AdmissionSupportStore: Send + Sync {
    /// Looks up a stored idempotent response. `Conflict` means the same
    /// key was reused with a different canonical request hash.
    async fn idempotency_find(
        &self,
        principal: &str,
        operation: &str,
        resource_kind: &str,
        resource_key: &str,
        idempotency_key: &str,
        request_hash: &str,
    ) -> CoreResult<IdempotencyLookup>;
    /// The store's diagnostic sink; `None` when it records no diagnostics.
    fn diagnostic_sink(&self) -> Option<std::sync::Arc<dyn crate::diagnostics::DiagnosticSink>>;
    async fn diagnostic_read(
        &self,
        kind: crate::diagnostics::SubjectKind,
        key: &str,
        actor: &Actor,
        now: i64,
    ) -> crate::diagnostics::DiagnosticsResult;
}
