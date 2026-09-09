//! Owned label replacements follow the pinned PATCH wire contract.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::Request;
use axum::http::{header, Method, StatusCode};
use axum::response::IntoResponse;
use axum::{routing::any, Json, Router};
use shaula_core::github::Label;
use shaula_core::ports::{AccessFailure, EffectOutcome, GitHubAccessPort};

use super::{app_client, wire_mock_router};

const PATH: &str = "/actions-service/_apis/runtime/runnerscalesets/42";

fn desired() -> Vec<Label> {
    vec![Label {
        name: "linux".into(),
        label_type: "Customer".into(),
    }]
}

fn view(labels: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "id": 42, "name": "shaula-x64", "runnerGroupId": 7,
        "runnerGroupName": "Default", "labels": labels,
    })
}

#[tokio::test]
async fn patch_only_replaces_labels_and_preserves_their_types() {
    for labels in [desired(), vec![Label::system("shaula-x64").unwrap()]] {
        let expected = serde_json::json!({"labels": labels});
        let expected_view = view(serde_json::json!(labels
            .iter()
            .map(|label| serde_json::json!({
                "name": label.name, "type": label.label_type.to_ascii_lowercase()
            }))
            .collect::<Vec<_>>()));
        let routes = Router::new().route(
            PATH,
            any(move |request: Request| {
                let expected = expected.clone();
                let response = expected_view.clone();
                async move {
                    assert_eq!(request.method(), Method::PATCH);
                    assert_eq!(request.uri().query(), Some("api-version=6.0-preview"));
                    assert_eq!(request.headers()[header::CONTENT_TYPE], "application/json");
                    assert!(request.headers()[header::AUTHORIZATION]
                        .to_str()
                        .unwrap()
                        .starts_with("Bearer "));
                    let bytes = axum::body::to_bytes(request.into_body(), 8192)
                        .await
                        .unwrap();
                    assert_eq!(
                        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
                        expected
                    );
                    Json(response)
                }
            }),
        );
        let client = app_client(wire_mock_router::spawn_mock_github_with_routes(routes).await);
        let result = client
            .update_scale_set_labels(42, &labels)
            .await
            .unwrap()
            .definite()
            .unwrap();
        assert_eq!(result.id, 42);
        assert_eq!(result.name, "shaula-x64");
        assert_eq!(result.runner_group_id, 7);
        assert_eq!(result.labels, labels);
    }
}

#[tokio::test]
async fn expired_admin_response_rechecks_authority_and_retries_patch_once() {
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    let routes = Router::new().route(
        PATH,
        any(move || {
            let seen = seen.clone();
            async move {
                if seen.fetch_add(1, Ordering::SeqCst) == 0 {
                    StatusCode::UNAUTHORIZED.into_response()
                } else {
                    Json(view(serde_json::json!(desired()))).into_response()
                }
            }
        }),
    );
    let client = app_client(wire_mock_router::spawn_mock_github_with_routes(routes).await);
    assert!(matches!(
        client
            .update_scale_set_labels(42, &desired())
            .await
            .unwrap(),
        EffectOutcome::Definite(_)
    ));
    assert_eq!(requests.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn patch_rejections_are_classified_without_unbounded_retries() {
    for status in [401, 403, 404, 429, 500] {
        let requests = Arc::new(AtomicUsize::new(0));
        let seen = requests.clone();
        let routes = Router::new().route(
            PATH,
            any(move || {
                let seen = seen.clone();
                async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    let mut response = StatusCode::from_u16(status).unwrap().into_response();
                    if status == 429 {
                        response
                            .headers_mut()
                            .insert(header::RETRY_AFTER, "7".parse().unwrap());
                    }
                    response
                }
            }),
        );
        let client = app_client(wire_mock_router::spawn_mock_github_with_routes(routes).await);
        let failure = client
            .update_scale_set_labels(42, &desired())
            .await
            .unwrap_err();
        assert!(match status {
            401 => matches!(failure, AccessFailure::Unauthenticated),
            403 => matches!(failure, AccessFailure::PermissionDenied),
            404 => matches!(failure, AccessFailure::TargetHiddenOrNotFound),
            429 =>
                matches!(failure, AccessFailure::RateLimited { retry_after: Some(delay) } if delay.as_secs() == 7),
            _ => matches!(failure, AccessFailure::Unavailable { .. }),
        });
        assert_eq!(
            requests.load(Ordering::SeqCst),
            if status == 401 { 2 } else { 1 }
        );
    }
}

#[tokio::test]
async fn patch_does_not_follow_redirects_or_accept_malformed_labels() {
    let redirected = Arc::new(AtomicUsize::new(0));
    let seen = redirected.clone();
    let routes = Router::new()
        .route(
            PATH,
            any(|| async {
                (
                    StatusCode::TEMPORARY_REDIRECT,
                    [(header::LOCATION, "/redirected")],
                )
            }),
        )
        .route(
            "/redirected",
            any(move || {
                let seen = seen.clone();
                async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    StatusCode::OK
                }
            }),
        );
    let client = app_client(wire_mock_router::spawn_mock_github_with_routes(routes).await);
    assert!(client
        .update_scale_set_labels(42, &desired())
        .await
        .is_err());
    assert_eq!(redirected.load(Ordering::SeqCst), 0);

    for body in [
        "{".to_string(),
        view(serde_json::json!([{"name": "linux", "type": "future-type"}])).to_string(),
    ] {
        let routes = Router::new().route(
            PATH,
            any(move || {
                let body = body.clone();
                async move { ([(header::CONTENT_TYPE, "application/json")], body) }
            }),
        );
        let client = app_client(wire_mock_router::spawn_mock_github_with_routes(routes).await);
        assert!(matches!(
            client.update_scale_set_labels(42, &desired()).await,
            Err(AccessFailure::Unavailable { .. })
        ));
    }
}

#[tokio::test]
async fn interrupted_patch_response_never_proves_an_update() {
    let routes = Router::new().route(
        PATH,
        any(|| async {
            axum::body::Body::from_stream(futures::stream::once(async {
                Err::<String, _>(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "lost reply",
                ))
            }))
        }),
    );
    let client = app_client(wire_mock_router::spawn_mock_github_with_routes(routes).await);
    assert!(!matches!(
        client.update_scale_set_labels(42, &desired()).await,
        Ok(EffectOutcome::Definite(_))
    ));
}
