use super::App;
use axum::{
    extract::{Path, Request, State},
    http::{header, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use std::time::Duration;

const MAX_BYTES: usize = 1024 * 1024;
const GROUP: &str = "Terraform apply (runner provisioning)";

pub(super) async fn handle(
    State(app): State<App>,
    Path(generation_id): Path<String>,
    request: Request,
) -> Response {
    if request.method() != Method::GET || request.uri().query().is_some() {
        return unknown().await;
    }
    if request
        .headers()
        .get_all(header::AUTHORIZATION)
        .iter()
        .count()
        != 1
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(token) = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|v| (32..=512).contains(&v.len()))
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Ok(_permit) = app.permits.try_acquire() else {
        return retry(StatusCode::TOO_MANY_REQUESTS);
    };
    let now = chrono::Utc::now().timestamp();
    match tokio::time::timeout(
        Duration::from_secs(5),
        app.authorizer.authorize(&generation_id, token, now),
    )
    .await
    {
        Ok(Ok(true)) => {}
        Ok(Ok(false)) => return StatusCode::UNAUTHORIZED.into_response(),
        _ => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
    if !app.rate.allow(token).await {
        return retry(StatusCode::TOO_MANY_REQUESTS);
    }
    let projection = match tokio::time::timeout(
        Duration::from_secs(5),
        app.logs.setup_projection(&generation_id),
    )
    .await
    {
        Ok(Ok(value)) => value,
        _ => return retry(StatusCode::SERVICE_UNAVAILABLE),
    };
    if projection.status == "pending" {
        return retry(StatusCode::ACCEPTED);
    }
    let detail = if projection.status == "ready" {
        projection
            .detail
            .unwrap_or_else(|| "[Shaula: setup info unavailable]".into())
    } else {
        "[Shaula: setup info unavailable or withheld; inspect authorized operation logs]".into()
    };
    render(detail)
}

#[derive(Serialize)]
struct Entry<'a> {
    #[serde(rename = "Group")]
    group: &'a str,
    #[serde(rename = "Detail")]
    detail: &'a str,
}

fn render(mut detail: String) -> Response {
    let mut raw = serde_json::to_vec(&[Entry {
        group: GROUP,
        detail: &detail,
    }]);
    if raw.as_ref().is_ok_and(|v| v.len() > MAX_BYTES) || detail.lines().count() > 20000 {
        let head: String = detail
            .lines()
            .take(5000)
            .collect::<Vec<_>>()
            .join("\n")
            .chars()
            .take(32768)
            .collect();
        let tail_lines = detail
            .lines()
            .rev()
            .take(5000)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        let tail: String = tail_lines
            .chars()
            .rev()
            .take(32768)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        detail =
            format!("{head}\n[Shaula: setup info truncated; full approved log in Web UI]\n{tail}");
        raw = serde_json::to_vec(&[Entry {
            group: GROUP,
            detail: &detail,
        }]);
    }
    match raw {
        Ok(raw) if raw.len() <= MAX_BYTES => {
            ([(header::CONTENT_TYPE, "application/json")], raw).into_response()
        }
        _ => Json([Entry {
            group: GROUP,
            detail: "[Shaula: setup info unavailable]",
        }])
        .into_response(),
    }
}

fn retry(status: StatusCode) -> Response {
    (status, [(header::RETRY_AFTER, "1")]).into_response()
}

pub(super) async fn unknown() -> Response {
    StatusCode::NOT_FOUND.into_response()
}

pub(super) async fn safe_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("private, no-store"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        header::HeaderValue::from_static("nosniff"),
    );
    response
}
