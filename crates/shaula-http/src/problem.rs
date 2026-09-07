//! Sanitized problem responses. Request bodies, secrets, idempotency keys
//! and provider diagnostics never appear in any error path.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use shaula_core::registry::MutationError;

/// Bounded problem detail with a stable `code`.
#[derive(serde::Serialize)]
pub struct Problem {
    pub status: u16,
    pub code: String,
    pub detail: String,
}

impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, Json(serde_json::json!(&self))).into_response()
    }
}

pub fn problem(status: StatusCode, code: &str, detail: impl Into<String>) -> Problem {
    Problem {
        status: status.as_u16(),
        code: code.to_string(),
        detail: detail.into(),
    }
}

/// Maps registry mutation errors onto the status contract.
pub fn mutation_problem(error: &MutationError) -> Problem {
    match error {
        MutationError::PreconditionRequired => problem(
            StatusCode::PRECONDITION_REQUIRED,
            "PreconditionRequired",
            "conditional headers required for this mutation",
        ),
        MutationError::PreconditionFailed { current } => problem(
            StatusCode::PRECONDITION_FAILED,
            "PreconditionFailed",
            format!(
                "stale precondition; current revision metadata: {}/{}",
                current.0, current.1
            ),
        ),
        MutationError::IdentityConflict => problem(
            StatusCode::CONFLICT,
            "Conflict",
            "immutable identity change rejected",
        ),
        MutationError::IdempotencyConflict => problem(
            StatusCode::CONFLICT,
            "IdempotencyConflict",
            "idempotency key reused with different request content",
        ),
        MutationError::Unprocessable { summary, .. } => problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Unprocessable",
            summary.clone(),
        ),
        MutationError::NotFound => problem(StatusCode::NOT_FOUND, "NotFound", "resource not found"),
        MutationError::Gone { .. } => problem(StatusCode::GONE, "Gone", "terminal tombstone"),
        MutationError::RetirementBlocked { reason } => problem(
            StatusCode::CONFLICT,
            "ResourceInUse",
            format!("retirement blocked: {reason}"),
        ),
        MutationError::TooManyRequests { retry_after_secs } => {
            let mut p = problem(
                StatusCode::TOO_MANY_REQUESTS,
                "RateLimited",
                "admission limit reached",
            );
            p.detail = format!("retry after {retry_after_secs}s");
            p
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping_contract() {
        assert_eq!(
            mutation_problem(&MutationError::PreconditionRequired).status,
            428
        );
        assert_eq!(
            mutation_problem(&MutationError::IdentityConflict).status,
            409
        );
        assert_eq!(
            mutation_problem(&MutationError::Unprocessable {
                reason: shaula_core::error::ReasonCode::SpecInvalid,
                summary: "bad".into()
            })
            .status,
            422
        );
        assert_eq!(mutation_problem(&MutationError::NotFound).status, 404);
        assert_eq!(
            mutation_problem(&MutationError::Gone {
                tombstone: "t".into()
            })
            .status,
            410
        );
        assert_eq!(
            mutation_problem(&MutationError::RetirementBlocked {
                reason: "in use".into()
            })
            .status,
            409
        );
    }

    #[test]
    fn problems_never_carry_provider_text() {
        let p = mutation_problem(&MutationError::Unprocessable {
            reason: shaula_core::error::ReasonCode::TemplateInvalid,
            summary: "manifest invalid".into(),
        });
        assert!(
            !p.detail.contains("terraform provider crashed"),
            "sanitized only"
        );
    }
}
