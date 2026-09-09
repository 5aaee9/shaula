//! HTTPS identity-provider fixture. Used only by integration test binaries.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use axum::{
    extract::{Query, State},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde_json::{json, Value};
use shaula_core::secret::SecretString;
use shaula_http::oidc::{Grant, Oidc, OidcConfig};
use std::sync::{Arc, OnceLock};
mod token;
pub use token::TokenControl;

pub const SCOPES: &str = "fleet.read logs.read fleet.write fleet.retire template.read template.publish template.attest template.retire auth.read auth.write auth.retire";
pub const KEY: &str = include_str!("../../../shaula-scaleset/tests/fixtures/app.private.pem");
pub const MODULUS: &str = "zBswup1vHos8WZhq32wBqrmCHTFbszkB0QSXNUF4-hJcHHqwrCMzRrEdPk9F5pUmE9oqtVoj1OX8UShriZfwSEx49J3WLcopKMJ3H4VILiqV2oqWt8LOGeUShzGsDpw5ulfPFDOjzTY72CHQscBlDD34Tj38OQkiEXlrYKj5fdBJ66GHVfv4zpARR7H8Yz9g1_QgHwdI9C-krawkJTnpgtaN9islh0keayWd7JgS7ygbeZRB-Ad25gxeXFpsdL-8T9w5c2w3tplJ1ob4Gb8MrSBrLo6qX0sSGKa-ozgX0AWkY3g041qWLuy7Ukf0PsoOR9o0RM6M9Uw9NczpYYtfjw";

pub struct Provider {
    pub issuer: String,
    pub certificate: Vec<u8>,
    pub requests: Arc<std::sync::Mutex<Vec<Value>>>,
    pub control: Arc<std::sync::Mutex<Control>>,
}

#[derive(Default)]
pub struct Control {
    pub offline: bool,
    pub keys_offline: bool,
    pub metadata: serde_json::Map<String, Value>,
    pub keys: Option<Value>,
    pub discovery_reads: usize,
    pub key_reads: usize,
    pub refresh_requests: usize,
    pub tokens: TokenControl,
}

impl Provider {
    pub fn config(&self, origin: &str) -> OidcConfig {
        OidcConfig::new(
            self.issuer.clone(),
            "web".into(),
            SecretString::new("test-secret"),
            origin.into(),
            "api".into(),
            vec![Grant {
                issuer: self.issuer.clone(),
                subject: "ops".into(),
                scopes: SCOPES.split_whitespace().map(str::to_owned).collect(),
            }],
        )
        .unwrap()
    }
    pub async fn oidc(&self) -> Arc<Oidc> {
        Oidc::discover_with_roots(
            self.config("https://shaula.example"),
            vec![reqwest::Certificate::from_pem(&self.certificate).unwrap()],
        )
        .await
        .unwrap()
    }
}

pub fn provider() -> &'static Provider {
    static PROVIDER: OnceLock<Provider> = OnceLock::new();
    PROVIDER.get_or_init(start_provider)
}

pub fn start_provider() -> Provider {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let issuer = format!(
        "https://localhost:{}",
        listener.local_addr().unwrap().port()
    );
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let certificate = cert.cert.pem().into_bytes();
    let private_key = cert.key_pair.serialize_pem().into_bytes();
    let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
    let control = Arc::new(std::sync::Mutex::new(Control::default()));
    let state = Fixture {
        issuer: issuer.clone(),
        requests: requests.clone(),
        codes: Arc::default(),
        refresh_grants: Arc::default(),
        control: control.clone(),
    };
    let pem = certificate.clone();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let _ = rustls::crypto::ring::default_provider().install_default();
            let tls = axum_server::tls_rustls::RustlsConfig::from_pem(pem, private_key)
                .await
                .unwrap();
            let app = Router::new()
                .route("/.well-known/openid-configuration", get(metadata))
                .route("/jwks", get(keys))
                .route("/authorize", get(authorize))
                .route("/token", post(token::exchange))
                .with_state(state);
            axum_server::from_tcp_rustls(listener, tls)
                .serve(app.into_make_service())
                .await
                .unwrap();
        });
    });
    Provider {
        issuer,
        certificate,
        requests,
        control,
    }
}

#[derive(Clone)]
struct Fixture {
    issuer: String,
    requests: Arc<std::sync::Mutex<Vec<Value>>>,
    codes: Arc<std::sync::Mutex<std::collections::HashMap<String, Value>>>,
    refresh_grants: Arc<std::sync::Mutex<std::collections::HashMap<String, Value>>>,
    control: Arc<std::sync::Mutex<Control>>,
}

async fn metadata(State(s): State<Fixture>) -> Response {
    let mut control = s.control.lock().unwrap();
    control.discovery_reads += 1;
    if control.offline {
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let mut value = json!({"issuer":s.issuer, "authorization_endpoint":format!("{}/authorize",s.issuer),
        "token_endpoint":format!("{}/token",s.issuer), "jwks_uri":format!("{}/jwks",s.issuer),
        "response_types_supported":["code"], "subject_types_supported":["public"],
        "id_token_signing_alg_values_supported":["RS256"], "token_endpoint_auth_methods_supported":["client_secret_basic"],
        "scopes_supported":["openid","profile"], "code_challenge_methods_supported":["S256"]});
    value
        .as_object_mut()
        .unwrap()
        .extend(control.metadata.clone());
    Json(value).into_response()
}
async fn keys(State(s): State<Fixture>) -> Response {
    let mut control = s.control.lock().unwrap();
    control.key_reads += 1;
    if control.offline || control.keys_offline {
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    Json(control.keys.clone().unwrap_or_else(|| json!({"keys":[{"kty":"RSA", "kid":"test-key", "use":"sig", "alg":"RS256", "n":MODULUS,"e":"AQAB"}]}))).into_response()
}
async fn authorize(
    State(s): State<Fixture>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Redirect {
    assert_eq!(params["code_challenge_method"], "S256");
    let code = uuid_like();
    s.codes.lock().unwrap().insert(code.clone(), json!(params));
    let mut target = url::Url::parse(&params["redirect_uri"]).unwrap();
    target
        .query_pairs_mut()
        .append_pair("code", &code)
        .append_pair("state", &params["state"]);
    Redirect::temporary(target.as_str())
}
fn uuid_like() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        .to_string()
}
fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn claims(issuer: &str, audience: &str, subject: &str, scopes: &str) -> Value {
    let now = jsonwebtoken::get_current_timestamp();
    json!({"iss":issuer,"sub":subject,"aud":audience,"iat":now,"exp":now+3600,
        "client_id":"automation", "jti":"test-id", "name":"Test operator", "scope":scopes})
}
pub fn sign(claims: Value, typ: &str, kid: &str) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.typ = Some(typ.into());
    header.kid = Some(kid.into());
    jsonwebtoken::encode(
        &header,
        &claims,
        &EncodingKey::from_rsa_pem(KEY.as_bytes()).unwrap(),
    )
    .unwrap()
}
pub fn bearer(scopes: &str) -> String {
    format!(
        "Bearer {}",
        sign(
            claims(&provider().issuer, "api", "ops", scopes),
            "at+jwt",
            "test-key"
        )
    )
}
pub fn config() -> OidcConfig {
    config_at("https://shaula.example")
}
pub fn config_at(origin: &str) -> OidcConfig {
    OidcConfig::new(
        provider().issuer.clone(),
        "web".into(),
        SecretString::new("test-secret"),
        origin.into(),
        "api".into(),
        vec![Grant {
            issuer: provider().issuer.clone(),
            subject: "ops".into(),
            scopes: SCOPES.split_whitespace().map(str::to_owned).collect(),
        }],
    )
    .unwrap()
}
pub async fn oidc() -> Arc<Oidc> {
    Oidc::discover_with_roots(
        config(),
        vec![reqwest::Certificate::from_pem(&provider().certificate).unwrap()],
    )
    .await
    .unwrap()
}
