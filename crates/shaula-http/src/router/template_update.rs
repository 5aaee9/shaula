//! A strict, non-secret update payload over normal conditional publication.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Deserializer};
use shaula_core::registry::{Scope, TemplateProfileUpdate};

use super::{accepted_response, idempotency_header, parse_if_match, require_scope, AppState};
use crate::problem::{mutation_problem, problem};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateDto {
    artifact_digest: String,
    engine_ref: String,
    #[serde(default, deserialize_with = "explicit_policy")]
    fleet_input_policy: Option<serde_json::Map<String, serde_json::Value>>,
}

// Missing means inherit. Explicit null is not an instruction to inherit.
fn explicit_policy<'de, D>(
    deserializer: D,
) -> Result<Option<serde_json::Map<String, serde_json::Value>>, D::Error>
where
    D: Deserializer<'de>,
{
    serde_json::Map::deserialize(deserializer).map(Some)
}

pub(super) async fn update(
    State(state): State<AppState>,
    Path(profile_key): Path<String>,
    auth: crate::oidc::Authenticated,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let actor = auth.actor;
    if let Err(response) = require_scope(&actor, Scope::TemplatePublish) {
        return response;
    }
    let Ok(dto) = serde_json::from_slice::<UpdateDto>(&body) else {
        return problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "SpecInvalid",
            "template update rejected by strict validation",
        )
        .into_response();
    };
    let if_match = match parse_if_match(&headers) {
        Ok(Some(expected)) => expected,
        Ok(None) => {
            return problem(
                StatusCode::PRECONDITION_REQUIRED,
                "PreconditionRequired",
                "If-Match required for template update",
            )
            .into_response();
        }
        Err(response) => return response,
    };
    if headers.contains_key("if-none-match") {
        return problem(
            StatusCode::BAD_REQUEST,
            "SpecInvalid",
            "If-None-Match is not supported for template updates",
        )
        .into_response();
    }
    let idempotency_key = match idempotency_header(&headers) {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .profiles
        .template_update(
            &actor,
            &profile_key,
            TemplateProfileUpdate {
                artifact_digest: dto.artifact_digest,
                engine_ref: dto.engine_ref,
                fleet_input_policy: dto.fleet_input_policy,
            },
            Some(if_match),
            idempotency_key,
        )
        .await
    {
        Ok(Ok(accepted)) => accepted_response(&accepted),
        Ok(Err(mutation)) => mutation_problem(&mutation).into_response(),
        Err(error) => {
            problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", error.summary).into_response()
        }
    }
}
