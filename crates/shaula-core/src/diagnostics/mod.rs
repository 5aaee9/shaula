//! Disposable explanations, never lifecycle authority.
pub use shaula_api_types::diagnostics::*;
mod catalog;
mod observation;
mod question;
pub use catalog::Code;
pub use observation::*;
pub use question::{capacity, timestamp, QuestionExt, QuestionStages};

#[async_trait::async_trait]
pub trait DiagnosticsReadPort: Send + Sync {
    async fn diagnostics(
        &self,
        kind: SubjectKind,
        key: &str,
        actor: &crate::registry::Actor,
        now: i64,
    ) -> DiagnosticsResult;
}

#[cfg(test)]
mod tests;

/// Read failures are separate from mutation reason codes.
#[derive(Debug, thiserror::Error)]
pub enum DiagnosticsReadError {
    #[error("diagnostics unavailable")]
    Unavailable,
    #[error("diagnostic subject has been removed")]
    Gone,
}
pub type DiagnosticsResult = Result<Option<DiagnosticReportV1>, DiagnosticsReadError>;
