use crate::{ForgejoClient, ForgejoScope};
use axum::{
    extract::State,
    http::{StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde_json::json;
use shaula_core::jobs::ForgejoTaskConclusion;
use std::sync::{Arc, Mutex};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Default)]
struct Fixture {
    paths: Arc<Mutex<Vec<String>>>,
    repository_id: Option<u64>,
    owner_id: Option<u64>,
    denied: bool,
    duplicate: bool,
    endless: bool,
}
async fn handler(State(state): State<Fixture>, uri: Uri) -> Response {
    if let Ok(mut paths) = state.paths.lock() {
        paths.push(uri.to_string());
    }
    match uri.path() {
        "/forgejo/api/v1/orgs/team" => Json(json!({"id":9})).into_response(),
        "/forgejo/api/v1/repositories/7" => {
            Json(json!({"id":state.repository_id.unwrap_or(7),"name":"repo","owner":{"id":state.owner_id.unwrap_or(9),"login":"owner"}})).into_response()
        }
        "/forgejo/api/v1/repos/owner/repo/actions/tasks" => {
            if state.denied { return StatusCode::FORBIDDEN.into_response(); }
            if state.endless {
                let page = uri.query().and_then(|q| url::form_urlencoded::parse(q.as_bytes()).find(|(k,_)| k=="page"))
                    .and_then(|(_,v)|v.parse::<u64>().ok()).unwrap_or(1);
                return Json(json!({"workflow_runs": (0..3).map(|offset| json!({"id":page*100+offset,"status":"success","run_number":1,"workflow_id":"test.yml"})).collect::<Vec<_>>()})).into_response();
            }
            if uri.query().is_some_and(|q| q.contains("page=2")) {
                Json(json!({"total_count":4,"workflow_runs":[
                    {"id":if state.duplicate {39} else {42},"status":"success","run_number":8,"workflow_id":"test.yml","url":"https://untrusted.test/ignored"}
                ]})).into_response()
            } else {
                Json(json!({"total_count":4,"workflow_runs":[
                    {"id":39,"status":"failure","run_number":8,"workflow_id":"test.yml"},
                    {"id":40,"status":"running","run_number":8,"workflow_id":"test.yml"},
                    {"id":41,"status":"future-status","run_number":8,"workflow_id":"test.yml"}
                ]}))
                .into_response()
            }
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn serve(
    state: &Fixture,
) -> TestResult<(String, tokio::task::JoinHandle<std::io::Result<()>>)> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}/forgejo/", listener.local_addr()?);
    let app = Router::new()
        .fallback(get(handler))
        .with_state(state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    Ok((base, server))
}

#[tokio::test]
async fn exact_task_history_ignores_names_other_tasks_and_response_urls() -> TestResult {
    let state = Fixture::default();
    let (base, server) = serve(&state).await?;
    let client = ForgejoClient::new(&base, "canary", ForgejoScope::Instance)?.with_page_size(3)?;
    let result = client.task_results(7, &[40, 41, 42]).await?;
    server.abort();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].repository_id, 7);
    assert_eq!(result[0].task_id, 42);
    assert_eq!(result[0].conclusion, ForgejoTaskConclusion::Success);
    assert_eq!(
        result[0].run_url,
        format!("{base}owner/repo/actions/runs/8")
    );
    assert_eq!(state.paths.lock().map_err(|_| "paths poisoned")?.len(), 3);
    Ok(())
}

#[tokio::test]
async fn task_history_refuses_changed_repository_scope_denials_and_duplicate_pages() -> TestResult {
    for state in [
        Fixture {
            repository_id: Some(8),
            ..Default::default()
        },
        Fixture {
            owner_id: Some(10),
            ..Default::default()
        },
        Fixture {
            denied: true,
            ..Default::default()
        },
        Fixture {
            duplicate: true,
            ..Default::default()
        },
    ] {
        let (base, server) = serve(&state).await?;
        let client =
            ForgejoClient::new(&base, "canary", ForgejoScope::Organization("team".into()))?
                .with_expected_scope_identity(9)?
                .with_page_size(3)?;
        let result = client.task_results(7, &[42]).await;
        server.abort();
        assert!(result.is_err());
        if state.repository_id.is_some() || state.owner_id.is_some() {
            assert!(!state
                .paths
                .lock()
                .map_err(|_| "paths poisoned")?
                .iter()
                .any(|path| path.contains("/tasks")));
        }
    }
    Ok(())
}

#[tokio::test]
async fn task_history_is_bounded_and_missing_task_ids_never_authorize_a_result() -> TestResult {
    let state = Fixture {
        endless: true,
        ..Default::default()
    };
    let (base, server) = serve(&state).await?;
    let client = ForgejoClient::new(&base, "canary", ForgejoScope::Instance)?.with_page_size(3)?;
    assert!(client.task_results(7, &[]).await?.is_empty());
    assert!(client.task_results(7, &[0]).await.is_err());
    assert!(client.task_results(0, &[42]).await.is_err());
    assert!(state.paths.lock().map_err(|_| "paths poisoned")?.is_empty());
    let result = client.task_results(7, &[42]).await?;
    server.abort();
    assert!(result.is_empty());
    assert_eq!(state.paths.lock().map_err(|_| "paths poisoned")?.len(), 11);
    Ok(())
}
