use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use axum::{body::Body, http::Request};
use base64::{engine::general_purpose::STANDARD, Engine};
use http_body_util::BodyExt;
use shaula_core::state_backend::{
    LockId, LockInfo, StateAccess, StateBackend, StateCapability, StateDocument, StateError,
    StateResult, StateSnapshot,
};
use tower::ServiceExt;
use uuid::Uuid;

use super::*;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
const DOCUMENT: &[u8] =
    br#"{"version":4,"lineage":"lineage","serial":0,"resources":[],"outputs":{}}"#;

struct Backend {
    access: StateAccess,
    reads: AtomicUsize,
    writes: AtomicUsize,
}

impl Backend {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            access: StateAccess {
                generation_id: Uuid::new_v4(),
                capability: StateCapability::issue(),
            },
            reads: AtomicUsize::new(0),
            writes: AtomicUsize::new(0),
        })
    }

    fn uri(&self) -> String {
        format!(
            "/internal/v1/generations/{}/state",
            self.access.generation_id
        )
    }

    fn basic(&self) -> String {
        format!(
            "Basic {}",
            STANDARD.encode(format!("shaula-state:{}", self.access.capability.expose()))
        )
    }

    fn request(&self, method: &str, suffix: &str, body: Body) -> TestResult<Request<Body>> {
        Ok(Request::builder()
            .method(method)
            .uri(format!("{}{suffix}", self.uri()))
            .header("Authorization", self.basic())
            .body(body)?)
    }
}

#[async_trait]
impl StateBackend for Backend {
    async fn authenticate(&self, access: &StateAccess) -> StateResult<()> {
        if access.generation_id != self.access.generation_id
            || !access
                .capability
                .matches(&self.access.capability.verifier())
        {
            return Err(StateError::Unauthorized);
        }
        Ok(())
    }
    async fn read(&self, access: &StateAccess) -> StateResult<Option<StateSnapshot>> {
        self.authenticate(access).await?;
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(Some(StateSnapshot {
            revision: 1,
            document: StateDocument::parse(DOCUMENT.to_vec())?,
        }))
    }
    async fn lock(&self, access: &StateAccess, info: LockInfo) -> StateResult<()> {
        self.authenticate(access).await?;
        if info.id().expose() == "conflict" {
            return Err(StateError::Locked(Box::new(LockInfo::parse(
                br#"{"ID":"owner","Who":"private"}"#,
            )?)));
        }
        Ok(())
    }
    async fn unlock(&self, access: &StateAccess, _id: &LockId) -> StateResult<()> {
        self.authenticate(access).await
    }
    async fn write(
        &self,
        access: &StateAccess,
        id: &LockId,
        document: StateDocument,
    ) -> StateResult<i64> {
        self.authenticate(access).await?;
        assert_eq!(id.expose(), "lock-id");
        assert_eq!(document.bytes(), DOCUMENT);
        self.writes.fetch_add(1, Ordering::Relaxed);
        Ok(1)
    }
}

#[tokio::test]
async fn wire_methods_raw_json_lock_conflict_and_default_denials() -> TestResult {
    let backend = Backend::new();
    let app = router(backend.clone());
    let response = app
        .clone()
        .oneshot(backend.request("GET", "", Body::empty())?)
        .await?;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    assert_eq!(response.headers()["content-type"], "application/json");
    assert_eq!(response.into_body().collect().await?.to_bytes(), DOCUMENT);
    for method in ["LOCK", "UNLOCK"] {
        let response = app
            .clone()
            .oneshot(backend.request(method, "", Body::from(r#"{"ID":"lock-id"}"#))?)
            .await?;
        assert_eq!(response.status(), 200);
    }
    let response = app
        .clone()
        .oneshot(backend.request("POST", "?ID=lock-id", Body::from(DOCUMENT))?)
        .await?;
    assert_eq!(response.status(), 200);
    assert_eq!(backend.writes.load(Ordering::Relaxed), 1);
    let response = app
        .clone()
        .oneshot(backend.request("LOCK", "", Body::from(r#"{"ID":"conflict"}"#))?)
        .await?;
    assert_eq!(response.status(), 423);
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    assert_eq!(
        LockInfo::parse(&response.into_body().collect().await?.to_bytes())?
            .id()
            .expose(),
        "owner"
    );
    for method in ["DELETE", "PUT", "PATCH", "HEAD", "OPTIONS"] {
        let response = app
            .clone()
            .oneshot(backend.request(method, "", Body::empty())?)
            .await?;
        assert_eq!(response.status(), 405);
        assert_eq!(response.headers()["allow"], "GET, POST, LOCK, UNLOCK");
    }
    for query in ["", "?ID=", "?ID=a&ID=b", "?ID=a&force=true", "?id=lock-id"] {
        let response = app
            .clone()
            .oneshot(backend.request("POST", query, Body::from(DOCUMENT))?)
            .await?;
        assert_eq!(response.status(), 400);
    }
    let request = Request::builder()
        .uri("/api/v1/fleets")
        .header("Authorization", backend.basic())
        .body(Body::empty())?;
    let response = app.oneshot(request).await?;
    assert_eq!(response.status(), 401);
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    Ok(())
}

#[tokio::test]
async fn authentication_precedes_body_poll_and_rejects_management_credentials() -> TestResult {
    let backend = Backend::new();
    let app = router(backend.clone());
    let polled = Arc::new(AtomicUsize::new(0));
    let observed = polled.clone();
    let body = Body::from_stream(futures::stream::poll_fn(move |_| {
        observed.fetch_add(1, Ordering::Relaxed);
        std::task::Poll::<Option<Result<bytes::Bytes, std::io::Error>>>::Pending
    }));
    let request = Request::builder()
        .method("POST")
        .uri(format!("{}?ID=lock-id", backend.uri()))
        .header("Content-Length", "999999999")
        .body(body)?;
    let response =
        tokio::time::timeout(Duration::from_secs(1), app.clone().oneshot(request)).await??;
    assert_eq!(response.status(), 401);
    assert_eq!(polled.load(Ordering::Relaxed), 0);
    for authorization in [
        "Bearer management-jwt".to_string(),
        format!("Bearer {}", backend.access.capability.expose()),
        format!("Basic {}", STANDARD.encode("shaula-state:control-token")),
        format!(
            "Basic {}",
            STANDARD.encode(format!("wrong:{}", backend.access.capability.expose()))
        ),
    ] {
        let request = Request::builder()
            .uri(backend.uri())
            .header("Authorization", authorization)
            .header("Cookie", "shaula_session=management-cookie")
            .body(Body::empty())?;
        assert_eq!(app.clone().oneshot(request).await?.status(), 401);
    }
    let request = Request::builder()
        .uri(format!("/internal/v1/generations/{}/state", Uuid::new_v4()))
        .header("Authorization", backend.basic())
        .body(Body::empty())?;
    assert_eq!(app.oneshot(request).await?.status(), 401);
    assert_eq!(backend.reads.load(Ordering::Relaxed), 0);
    Ok(())
}

#[tokio::test]
async fn malformed_and_oversized_bodies_do_not_reach_the_store() -> TestResult {
    let backend = Backend::new();
    let app = router(backend.clone());
    for (body, status) in [
        (b"not-json".to_vec(), 400),
        (b"{}".to_vec(), 400),
        (
            vec![b' '; shaula_core::state_backend::MAX_STATE_BYTES + 1],
            413,
        ),
    ] {
        let response = app
            .clone()
            .oneshot(backend.request("POST", "?ID=lock-id", Body::from(body))?)
            .await?;
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()["cache-control"], "private, no-store");
    }
    assert_eq!(backend.writes.load(Ordering::Relaxed), 0);
    Ok(())
}

#[tokio::test]
async fn private_server_binds_only_loopback_and_serves_real_http() -> TestResult {
    let backend = Backend::new();
    assert!(StateServer::bind("0.0.0.0:0".parse()?, backend.clone())
        .await
        .is_err());
    let server = StateServer::bind("127.0.0.1:0".parse()?, backend.clone()).await?;
    let address = server.local_addr()?;
    let (shutdown, stop) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(server.serve(async {
        let _ = stop.await;
    }));
    let client = reqwest::Client::builder().no_proxy().build()?;
    let response = client
        .get(format!("http://{address}{}", backend.uri()))
        .header("Authorization", backend.basic())
        .send()
        .await?;
    assert_eq!(response.status(), 200);
    assert_eq!(response.bytes().await?, DOCUMENT);
    shutdown.send(()).map_err(|_| "server stopped early")?;
    task.await??;
    Ok(())
}
