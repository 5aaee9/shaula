//! Scripted GitHub and Actions endpoints for real-adapter authorization tests.

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{routing::any, Json, Router};
use base64::Engine;
use shaula_core::auth_context::ResolvedAuthContext;
use shaula_core::auth_policy::AccountKind;
use shaula_core::github::GitHubTarget;
use shaula_core::ports::{Clock, SessionHandle};
use shaula_core::secret::SecretString;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU16, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use tokio::sync::Notify;

use crate::{Credential, ScalesetClient};

pub(super) struct TestClock(pub AtomicI64);
impl Clock for TestClock {
    fn now_unix_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

pub(super) struct Script {
    pub base: String,
    pub installation_reads: AtomicUsize,
    pub effects: AtomicUsize,
    pub cleanup_requests: AtomicUsize,
    pub token_bodies: RwLock<Vec<serde_json::Value>>,
    pub metadata_bearers: RwLock<Vec<String>>,
    pub registration_bearers: RwLock<Vec<String>>,
    pub installation: RwLock<serde_json::Value>,
    pub organization: RwLock<serde_json::Value>,
    pub repository: RwLock<serde_json::Value>,
    pub metadata_status: AtomicU16,
    pub groups_status: AtomicU16,
    pub groups_retry_status: AtomicU16,
    pub token_status: AtomicU16,
    pub registration_status: AtomicU16,
    pub effect_status: AtomicU16,
    pub groups_after_effect: AtomicU16,
    pub queue_status: AtomicU16,
    pub block_metadata_once: AtomicBool,
    pub metadata_started: Notify,
    pub metadata_release: Notify,
}

pub(super) struct Fixture {
    pub script: Arc<Script>,
    pub clock: Arc<TestClock>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Fixture {
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let script = Arc::new(Script {
            base,
            installation_reads: AtomicUsize::new(0),
            effects: AtomicUsize::new(0),
            cleanup_requests: AtomicUsize::new(0),
            token_bodies: RwLock::new(Vec::new()),
            metadata_bearers: RwLock::new(Vec::new()),
            registration_bearers: RwLock::new(Vec::new()),
            installation: RwLock::new(serde_json::json!({
                "id": 34, "app_id": 123,
                "account": {"id": 100, "login": "example", "type": "Organization"},
                "repository_selection": "all", "suspended_at": null,
                "permissions": {"metadata": "read", "administration": "write",
                    "organization_self_hosted_runners": "write"}
            })),
            organization: RwLock::new(serde_json::json!({"id": 100, "login": "example"})),
            repository: RwLock::new(serde_json::json!({"id": 700, "name": "proj",
                "owner": {"id": 100, "login": "example", "type": "Organization"}})),
            metadata_status: AtomicU16::new(200),
            groups_status: AtomicU16::new(200),
            groups_retry_status: AtomicU16::new(0),
            token_status: AtomicU16::new(201),
            registration_status: AtomicU16::new(201),
            effect_status: AtomicU16::new(200),
            groups_after_effect: AtomicU16::new(0),
            queue_status: AtomicU16::new(202),
            block_metadata_once: AtomicBool::new(false),
            metadata_started: Notify::new(),
            metadata_release: Notify::new(),
        });
        let router = Router::new()
            .fallback(any(handle))
            .with_state(script.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            script,
            clock: Arc::new(TestClock(AtomicI64::new(1_000_000))),
            task,
        }
    }

    pub fn client(&self, repository: bool, persisted: bool) -> ScalesetClient {
        self.client_with_issuer(repository, persisted, "123")
    }

    pub fn client_with_issuer(
        &self,
        repository: bool,
        persisted: bool,
        issuer: &str,
    ) -> ScalesetClient {
        let target = if repository {
            GitHubTarget::new_repository("example", "proj").unwrap()
        } else {
            GitHubTarget::organization("example").unwrap()
        };
        let expected = persisted.then(|| ResolvedAuthContext {
            profile_key: "shared".into(),
            revision: 1,
            github_host: "github.com".into(),
            app_id: "123".into(),
            account_id: 100,
            account_kind: AccountKind::Organization,
            login: "example".into(),
            installation_id: 34,
            target: target.clone(),
            organization_id: (!repository).then_some(100),
            repository_id: repository.then_some(700),
            repository_owner_id: repository.then_some(100),
        });
        ScalesetClient::with_local_servers(
            target,
            Credential::GitHubApp {
                client_id: issuer.into(),
                installation_id: 34,
                private_key: SecretString::new(
                    std::fs::read_to_string(
                        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("tests/fixtures/app.private.pem"),
                    )
                    .unwrap(),
                ),
            },
            self.script.base.clone(),
            self.clock.clone(),
            crate::client::production_http_client().unwrap(),
        )
        .with_expected_context(expected)
    }

    pub fn session(&self) -> SessionHandle {
        SessionHandle {
            session_id: "session".into(),
            message_queue_url: format!("{}/queue", self.script.base),
            message_queue_access_token: "queue-token".into(),
            initial_statistics: Default::default(),
        }
    }

    pub fn reads(&self) -> usize {
        self.script.installation_reads.load(Ordering::SeqCst)
    }
    pub fn advance(&self, ms: i64) {
        self.clock.0.fetch_add(ms, Ordering::SeqCst);
    }
}

async fn handle(State(s): State<Arc<Script>>, request: Request) -> Response {
    let path = request.uri().path().to_string();
    let bearer = request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default()
        .to_string();
    match path.as_str() {
        "/app/installations/34" => {
            s.installation_reads.fetch_add(1, Ordering::SeqCst);
            Json(s.installation.read().unwrap().clone()).into_response()
        }
        "/app/installations/34/access_tokens" => {
            let bytes = axum::body::to_bytes(request.into_body(), 16_384)
                .await
                .unwrap();
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            let metadata_only = body == serde_json::json!({"permissions": {"metadata": "read"}});
            s.token_bodies.write().unwrap().push(body);
            reply(
                s.token_status.load(Ordering::SeqCst),
                serde_json::json!({"token": if metadata_only { "metadata-token" } else { "runner-token" }, "expires_at": "2027-01-01T00:00:00Z"}),
            )
        }
        "/orgs/example" | "/repos/example/proj" => {
            s.metadata_bearers.write().unwrap().push(bearer);
            if s.block_metadata_once.swap(false, Ordering::SeqCst) {
                s.metadata_started.notify_one();
                s.metadata_release.notified().await;
            }
            let body = if path.starts_with("/orgs") {
                s.organization.read().unwrap().clone()
            } else {
                s.repository.read().unwrap().clone()
            };
            reply(s.metadata_status.load(Ordering::SeqCst), body)
        }
        "/orgs/example/actions/runners/registration-token"
        | "/repos/example/proj/actions/runners/registration-token" => {
            s.registration_bearers.write().unwrap().push(bearer);
            reply(
                s.registration_status.load(Ordering::SeqCst),
                serde_json::json!({"token": "registration"}),
            )
        }
        "/actions/runner-registration" => {
            let token = format!(
                "e30.{}.signature",
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"exp":1800003600}"#)
            );
            Json(serde_json::json!({"url": format!("{}/actions", s.base), "token": token}))
                .into_response()
        }
        "/actions/_apis/runtime/runnergroups/" => {
            let status = s.groups_status.load(Ordering::SeqCst);
            let next = s.groups_retry_status.swap(0, Ordering::SeqCst);
            if next != 0 {
                s.groups_status.store(next, Ordering::SeqCst);
            }
            reply(
                status,
                serde_json::json!({"count": 1, "value": [{"id": 7, "name": "Default"}]}),
            )
        }
        path if path.starts_with("/queue") => {
            reply(s.queue_status.load(Ordering::SeqCst), serde_json::json!({}))
        }
        path if path.starts_with("/actions/_apis/runtime/runnerscalesets") => {
            s.effects.fetch_add(1, Ordering::SeqCst);
            let next = s.groups_after_effect.swap(0, Ordering::SeqCst);
            if next != 0 {
                s.groups_status.store(next, Ordering::SeqCst);
            }
            let body = if path.ends_with("acquirejobs") {
                serde_json::json!({"count": 1, "value": [1]})
            } else if path.ends_with("generatejitconfig") {
                serde_json::json!({"encodedJITConfig": "jit-config", "runner": {"id": 77, "name": "runner", "runnerScaleSetId": 9}})
            } else if path.ends_with("sessions") {
                serde_json::json!({"sessionId": "a82c6d3f-0ea4-43fd-a6d8-3d48b8db60b1", "messageQueueUrl": format!("{}/queue", s.base), "messageQueueAccessToken": "queue-token", "statistics": {"totalAssignedJobs": 0}})
            } else {
                serde_json::json!({"id": 9, "name": "test", "runnerGroupId": 7})
            };
            reply(s.effect_status.load(Ordering::SeqCst), body)
        }
        "/actions/_apis/distributedtask/pools/0/agents/77" => {
            s.cleanup_requests.fetch_add(1, Ordering::SeqCst);
            StatusCode::NO_CONTENT.into_response()
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

fn reply(status: u16, body: serde_json::Value) -> Response {
    let mut response = (StatusCode::from_u16(status).unwrap(), Json(body)).into_response();
    if status == 429 {
        response
            .headers_mut()
            .insert("retry-after", "7".parse().unwrap());
    }
    response
}
