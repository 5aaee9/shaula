//! Template/Auth Profile route handlers.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;

use shaula_core::error::ReasonCode;
use shaula_core::registry::{AttestationPut, AuthProfilePut, Scope, TemplateProfilePut};

use crate::dto::{AttestationPutDto, AuthProfilePutDto, TemplateProfilePutDto};
use crate::problem::{mutation_problem, problem};
use crate::router::{
    accepted_response, idempotency_header, if_none_match_star, parse_if_match, require_scope,
    resource_version_headers, AppState,
};

pub(crate) async fn template_artifact_put(
    State(state): State<AppState>,
    Path(digest): Path<String>,
    auth: crate::oidc::Authenticated,
    body: Bytes,
) -> Response {
    let actor = auth.actor;
    if let Err(response) = require_scope(&actor, Scope::TemplatePublish) {
        return response;
    }
    if !digest.starts_with("sha256:") || digest.len() != 7 + 64 {
        return problem(
            StatusCode::BAD_REQUEST,
            "DigestInvalid",
            "digest must be sha256:<64 hex>",
        )
        .into_response();
    }
    if body.len() > state.body_limit {
        return problem(
            StatusCode::PAYLOAD_TOO_LARGE,
            "ArtifactTooLarge",
            "artifact body exceeds limit",
        )
        .into_response();
    }
    match state.artifact_publisher.publish(&body, &digest).await {
        Ok(size) => (
            StatusCode::CREATED,
            Json(serde_json::json!({"digest": digest, "sizeBytes": size})),
        )
            .into_response(),
        Err(e)
            if matches!(
                e.code,
                ReasonCode::StorageUnavailable | ReasonCode::Internal
            ) =>
        {
            problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal",
                "template library storage is unavailable",
            )
            .into_response()
        }
        Err(e) => problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "ArtifactRejected",
            e.summary,
        )
        .into_response(),
    }
}

pub(crate) async fn template_profile_put(
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
    let Ok(dto) = serde_json::from_slice::<TemplateProfilePutDto>(&body) else {
        return problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "SpecInvalid",
            "template profile rejected by strict validation",
        )
        .into_response();
    };
    let idempotency_key = match idempotency_header(&headers) {
        Ok(v) => v,
        Err(response) => return response,
    };
    // R9-02 (spec 0005 §3): conditional-write preconditions are parsed
    // and enforced — a stale ETag can never advance the desired head.
    let create = if_none_match_star(&headers);
    let if_match = match parse_if_match(&headers) {
        Ok(v) => v,
        Err(response) => return response,
    };
    if !create && if_match.is_none() {
        return problem(
            StatusCode::PRECONDITION_REQUIRED,
            "PreconditionRequired",
            "If-None-Match: * or If-Match required",
        )
        .into_response();
    }
    let payload = TemplateProfilePut {
        artifact_digest: dto.artifact_digest,
        engine_ref: dto.engine_ref,
        bindings: dto.bindings,
        fleet_input_policy: dto.fleet_input_policy,
    };
    match state
        .profiles
        .template_put(
            &actor,
            &profile_key,
            payload,
            create,
            if_match,
            idempotency_key,
        )
        .await
    {
        Ok(Ok(accepted)) => accepted_response(&accepted),
        Ok(Err(mutation)) => mutation_problem(&mutation).into_response(),
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}

pub(crate) async fn template_attestation_put(
    State(state): State<AppState>,
    Path((profile_key, revision, attestation_key)): Path<(String, i64, String)>,
    auth: crate::oidc::Authenticated,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let actor = auth.actor;
    if let Err(response) = require_scope(&actor, Scope::TemplateAttest) {
        return response;
    }
    // R10-05 (spec 0005 §5): attestation PUT REQUIRES `If-None-Match: *`
    // — attestations are create-only immutable records.
    if !if_none_match_star(&headers) {
        return problem(
            StatusCode::PRECONDITION_REQUIRED,
            "PreconditionRequired",
            "If-None-Match: * required for attestation",
        )
        .into_response();
    }
    let Ok(dto) = serde_json::from_slice::<AttestationPutDto>(&body) else {
        return problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "SpecInvalid",
            "attestation rejected by strict validation",
        )
        .into_response();
    };
    let payload = AttestationPut {
        attestation_key,
        subject: dto.subject,
        result: dto.result,
        evidence_digest: dto.evidence_digest,
        suite: (dto.suite.name, dto.suite.version),
        completed_at: dto.completed_at,
    };
    match state
        .profiles
        .attestation_put(&actor, &profile_key, revision, payload)
        .await
    {
        Ok(Ok(attestation_id)) => (
            StatusCode::CREATED,
            Json(serde_json::json!({"attestationId": attestation_id})),
        )
            .into_response(),
        Ok(Err(mutation)) => mutation_problem(&mutation).into_response(),
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}

pub(crate) async fn auth_profile_put(
    State(state): State<AppState>,
    Path(profile_key): Path<String>,
    auth: crate::oidc::Authenticated,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let actor = auth.actor;
    if let Err(response) = require_scope(&actor, Scope::AuthWrite) {
        return response;
    }
    let Ok(dto) = serde_json::from_slice::<AuthProfilePutDto>(&body) else {
        return problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "SpecInvalid",
            "auth profile rejected by strict validation",
        )
        .into_response();
    };
    let idempotency_key = match idempotency_header(&headers) {
        Ok(v) => v,
        Err(response) => return response,
    };
    let create = if_none_match_star(&headers);
    let if_match = match parse_if_match(&headers) {
        Ok(v) => v,
        Err(response) => return response,
    };
    if !create && if_match.is_none() {
        return problem(
            StatusCode::PRECONDITION_REQUIRED,
            "PreconditionRequired",
            "If-None-Match: * or If-Match required",
        )
        .into_response();
    }
    let payload = match build_auth_payload(dto) {
        Ok(payload) => payload,
        Err(response) => return response,
    };
    match state
        .profiles
        .auth_put(
            &actor,
            &profile_key,
            payload,
            create,
            if_match,
            idempotency_key,
        )
        .await
    {
        Ok(Ok(accepted)) => accepted_response(&accepted),
        Ok(Err(mutation)) => mutation_problem(&mutation).into_response(),
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}

pub(crate) fn build_auth_payload(dto: AuthProfilePutDto) -> Result<AuthProfilePut, Response> {
    use shaula_core::auth::AuthKind;
    let kind = match dto.kind.as_str() {
        "github_app" | "github-app" => AuthKind::GithubApp,
        "pat" => AuthKind::Pat,
        other => {
            return Err(problem(
                StatusCode::UNPROCESSABLE_ENTITY,
                "SpecInvalid",
                format!("unknown auth kind {other}"),
            )
            .into_response())
        }
    };
    let secret = match (&dto.private_key.flatten(), &dto.token.flatten()) {
        (Some(_), Some(_)) => {
            return Err(problem(
                StatusCode::UNPROCESSABLE_ENTITY,
                "SpecInvalid",
                "exactly one credential field required",
            )
            .into_response())
        }
        (Some(key), None) if kind == AuthKind::GithubApp => {
            shaula_core::secret::SecretString::new(key.clone())
        }
        (None, Some(token)) if kind == AuthKind::Pat => {
            shaula_core::secret::SecretString::new(token.clone())
        }
        _ => {
            return Err(problem(
                StatusCode::UNPROCESSABLE_ENTITY,
                "SpecInvalid",
                "credential bytes required for the declared kind",
            )
            .into_response())
        }
    };
    // F10: a v2 payload carrying ANY legacy member — value, empty, or
    // null — is a forbidden-member violation detected at this boundary.
    if dto.schema_version.flatten() == Some(2)
        && (dto.target_allowlist.is_present()
            || dto.installation_id.is_present()
            || dto.pat_principal.is_present())
    {
        return Err(problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "SpecInvalid",
            "schema_version 2 forbids installation_id, pat and target_allowlist fields",
        )
        .into_response());
    }
    // F10: legacy members flatten (null == absent, baseline behavior);
    // the v2 format validates presence itself inside service_auth_format.
    Ok(AuthProfilePut {
        kind,
        app_id: dto.app_id.flatten(),
        installation_id: dto.installation_id.flatten(),
        pat_identity: dto.pat_principal.flatten(),
        secret,
        allowlist: dto.target_allowlist.flatten(),
        schema_version: dto.schema_version.flatten(),
        target_policy: dto.target_policy.0.flatten(),
    })
}

pub(crate) async fn template_profile_get(
    State(state): State<AppState>,
    Path(profile_key): Path<String>,
    auth: crate::oidc::Authenticated,
) -> Response {
    let actor = auth.actor;
    if let Err(response) = require_scope(&actor, Scope::TemplateRead) {
        return response;
    }
    match state.profiles.template_get(&actor, &profile_key).await {
        Ok(Ok(view)) => (
            StatusCode::OK,
            resource_version_headers(&format!("{}:{}", view.incarnation, view.desired_revision)),
            Json(serde_json::json!({
                "key": view.key,
                "incarnation": view.incarnation,
                "desiredRevision": view.desired_revision,
                "activeRevision": view.active_revision,
                "status": view.status,
                "platform": view.platform,
                "bindingsContract": view.bindings_contract,
                "bindings_present": view.bindings_present,
            })),
        )
            .into_response(),
        Ok(Err(mutation)) => mutation_problem(&mutation).into_response(),
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}

// ─── R10-05: the remaining specified Profile read/retire routes ───
