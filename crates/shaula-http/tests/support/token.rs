//! Token endpoint behavior belongs only to the HTTPS test Provider.
use super::{base64_encode, claims, sign, uuid_like, Fixture, SCOPES};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Form, Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::{collections::HashMap, time::Duration};

#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct TokenControl {
    pub issue_refresh: bool,
    pub expires_in: Option<u64>,
    pub rotate_refresh: bool,
    pub omit_refresh_id_token: bool,
    pub omit_refresh_nonce: bool,
    pub refresh_status: Option<u16>,
    pub refresh_error: Option<String>,
    pub delay_ms: u64,
    pub login_claims: Map<String, Value>,
    pub refresh_claims: Map<String, Value>,
    pub refresh_response: Map<String, Value>,
    pub refresh_kid: Option<String>,
}

impl Default for TokenControl {
    fn default() -> Self {
        Self {
            issue_refresh: false,
            expires_in: None,
            rotate_refresh: true,
            omit_refresh_id_token: false,
            omit_refresh_nonce: false,
            refresh_status: None,
            refresh_error: None,
            delay_ms: 0,
            login_claims: Map::new(),
            refresh_claims: Map::new(),
            refresh_response: Map::new(),
            refresh_kid: None,
        }
    }
}

pub(super) async fn exchange(
    State(s): State<Fixture>,
    headers: HeaderMap,
    Form(params): Form<HashMap<String, String>>,
) -> Response {
    assert_eq!(headers["authorization"], "Basic d2ViOnRlc3Qtc2VjcmV0");
    s.requests.lock().unwrap().push(json!(params));
    let refresh = params["grant_type"] == "refresh_token";
    let (offline, control) = {
        let mut control = s.control.lock().unwrap();
        if refresh {
            control.refresh_requests += 1;
        }
        (control.offline, control.tokens.clone())
    };
    if offline {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    if !refresh {
        assert_eq!(params["grant_type"], "authorization_code");
        return login(&s, &params, &control).into_response();
    }
    let response = refresh_token(&s, &params, &control);
    // Construct and rotate first: disconnecting cannot undo a Provider exchange.
    tokio::time::sleep(Duration::from_millis(control.delay_ms)).await;
    response
}

fn login(s: &Fixture, params: &HashMap<String, String>, control: &TokenControl) -> Json<Value> {
    let code = s.codes.lock().unwrap().remove(&params["code"]).unwrap();
    use sha2::Digest;
    let challenge = base64_encode(&sha2::Sha256::digest(params["code_verifier"].as_bytes()));
    assert_eq!(challenge, code["code_challenge"]);
    assert_eq!(params["redirect_uri"], code["redirect_uri"]);
    let mut identity = claims(&s.issuer, "web", "ops", SCOPES);
    identity["nonce"] = code["nonce"].clone();
    patch(&mut identity, &control.login_claims);
    let mut response = json!({
        "access_token": "unused", "token_type": "Bearer",
        "id_token": sign(identity.clone(), "JWT", "test-key")
    });
    if let Some(seconds) = control.expires_in {
        response["expires_in"] = json!(seconds);
    }
    if control.issue_refresh {
        let token = format!("refresh-{}", uuid_like());
        s.refresh_grants.lock().unwrap().insert(
            token.clone(),
            json!({"identity": identity, "scope": code["scope"]}),
        );
        response["refresh_token"] = json!(token);
        response["scope"] = code["scope"].clone();
    }
    Json(response)
}

fn refresh_token(
    s: &Fixture,
    params: &HashMap<String, String>,
    control: &TokenControl,
) -> Response {
    if let Some(status) = control.refresh_status {
        return (
            StatusCode::from_u16(status).unwrap(),
            Json(json!({"error": control.refresh_error.as_deref().unwrap_or("temporarily_unavailable")})),
        )
            .into_response();
    }
    if let Some(error) = &control.refresh_error {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": error}))).into_response();
    }
    let token = &params["refresh_token"];
    let mut grants = s.refresh_grants.lock().unwrap();
    let Some(grant) = grants.get(token).cloned() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"invalid_grant"})),
        )
            .into_response();
    };
    let mut response =
        json!({"access_token":"refreshed-access", "token_type":"Bearer", "scope":grant["scope"]});
    if let Some(seconds) = control.expires_in {
        response["expires_in"] = json!(seconds);
    }
    if control.rotate_refresh {
        let replacement = format!("refresh-{}", uuid_like());
        grants.remove(token);
        grants.insert(replacement.clone(), grant.clone());
        response["refresh_token"] = json!(replacement);
    }
    drop(grants);
    if !control.omit_refresh_id_token {
        let mut identity = claims(&s.issuer, "web", "ops", SCOPES);
        for key in ["iss", "sub", "aud", "azp", "auth_time", "nonce"] {
            if let Some(value) = grant["identity"].get(key) {
                identity[key] = value.clone();
            }
        }
        if control.omit_refresh_nonce {
            identity.as_object_mut().unwrap().remove("nonce");
        }
        patch(&mut identity, &control.refresh_claims);
        response["id_token"] = json!(sign(
            identity,
            "JWT",
            control.refresh_kid.as_deref().unwrap_or("test-key")
        ));
    }
    patch(&mut response, &control.refresh_response);
    Json(response).into_response()
}

fn patch(target: &mut Value, patch: &Map<String, Value>) {
    let target = target.as_object_mut().unwrap();
    for (key, value) in patch {
        if value.is_null() {
            target.remove(key);
        } else {
            target.insert(key.clone(), value.clone());
        }
    }
}
