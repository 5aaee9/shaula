use super::*;
use axum::{
    extract::State,
    http::{Method, StatusCode, Uri},
    routing::any,
    Json, Router,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
#[derive(Clone)]
struct StateData {
    version: &'static str,
    mutations: Arc<AtomicUsize>,
}
struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn serve(
    version: &'static str,
) -> Result<(Server, ForgejoClient, Arc<AtomicUsize>), Box<dyn std::error::Error>> {
    let mutations = Arc::new(AtomicUsize::new(0));
    let app = Router::new().fallback(any(handler)).with_state(StateData {
        version,
        mutations: mutations.clone(),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/forgejo/", listener.local_addr()?);
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((
        Server(task),
        ForgejoClient::new(&url, "control-token", ForgejoScope::User)?,
        mutations,
    ))
}
async fn handler(
    State(state): State<StateData>,
    method: Method,
    uri: Uri,
) -> (StatusCode, Json<serde_json::Value>) {
    use serde_json::json;
    if method == Method::POST || method == Method::DELETE {
        state.mutations.fetch_add(1, Ordering::SeqCst);
    }
    let result = match uri.path() {
        "/forgejo/api/v1/version" => json!({"version":state.version}),
        "/forgejo/api/v1/user" => json!({"id":42}),
        "/forgejo/api/v1/user/actions/runners" => json!([]),
        _ => return (StatusCode::NOT_FOUND, Json(json!({}))),
    };
    (StatusCode::OK, Json(result))
}

#[tokio::test]
async fn unsupported_server_cannot_probe_or_register() -> TestResult {
    for version in ["14.0.3", "15.0.0-rc1", "development", "1.25.0"] {
        let (_server, client, mutations) = serve(version).await?;
        assert!(matches!(
            client.probe_authentication().await,
            Err(ForgejoError::UnsupportedServerVersion)
        ));
        assert!(matches!(
            client.register_runner("pool-g", None).await,
            Err(ForgejoError::UnsupportedServerVersion)
        ));
        assert_eq!(mutations.load(Ordering::SeqCst), 0);
    }
    Ok(())
}

#[tokio::test]
async fn supported_probe_preserves_prefix_version_and_scope_evidence() -> TestResult {
    let (_server, client, mutations) = serve("16.0.4").await?;
    let probe = client.probe_authentication().await?;
    assert_eq!(probe.server_version, "16.0.4");
    assert_eq!(probe.principal_id, Some(42));
    assert_eq!(probe.target_id, None);
    assert_eq!(probe.runner_count, 0);
    assert_eq!(probe.valid_until_unix_ms - probe.checked_at_unix_ms, 60_000);
    assert_eq!(mutations.load(Ordering::SeqCst), 0);
    Ok(())
}
