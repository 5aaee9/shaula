#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod common;
use axum::http::{HeaderValue, StatusCode};
use common::*;
use shaula_core::registry::ControlPlaneStore;
use tower::ServiceExt;

#[tokio::test]
async fn profile_create_replays_before_preconditions_and_retirement_is_durable() {
    let (app, store, _) = build_app_with_scan().await;
    let (digest, bytes) = fixture_artifact();
    let mut upload = authorized("PUT", &format!("/api/v1/template-artifacts/{digest}"), None);
    *upload.body_mut() = axum::body::Body::from(bytes);
    assert_eq!(
        app.clone().oneshot(upload).await.unwrap().status(),
        StatusCode::CREATED
    );
    for (kind, key, body) in [
        (
            "github-auth-profiles",
            "prod-app",
            AUTH_PUT_BODY.to_string(),
        ),
        (
            "template-profiles",
            "k8s-linux",
            TEMPLATE_PUT_BODY.replace("PLACEHOLDER", &digest),
        ),
    ] {
        let uri = format!("/api/v1/{kind}/{key}");
        let first = app
            .clone()
            .oneshot(put_with_idempotency(&uri, "put-1", body.clone()))
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::ACCEPTED);
        let etag = first.headers()["etag"].clone();
        assert_eq!(first.headers().get("shaula-resource-version"), Some(&etag));
        let view = app
            .clone()
            .oneshot(authorized("GET", &uri, None))
            .await
            .unwrap();
        assert_eq!(view.headers().get("shaula-resource-version"), Some(&etag));
        assert_eq!(view.headers().get("etag"), Some(&etag));
        let first_body = axum::body::to_bytes(first.into_body(), 1 << 20)
            .await
            .unwrap();
        let replay = app
            .clone()
            .oneshot(put_with_idempotency(&uri, "put-1", body.clone()))
            .await
            .unwrap();
        assert_eq!(replay.status(), StatusCode::ACCEPTED);
        assert_eq!(replay.headers().get("shaula-resource-version"), Some(&etag));
        assert_eq!(
            axum::body::to_bytes(replay.into_body(), 1 << 20)
                .await
                .unwrap(),
            first_body
        );
        assert_eq!(
            app.clone()
                .oneshot(authorized("DELETE", &uri, None))
                .await
                .unwrap()
                .status(),
            StatusCode::PRECONDITION_REQUIRED
        );
        let mut delete = authorized("DELETE", &uri, None);
        delete.headers_mut().insert("if-match", etag.clone());
        delete
            .headers_mut()
            .insert("idempotency-key", HeaderValue::from_static("delete-1"));
        let retired = app.clone().oneshot(delete).await.unwrap();
        assert_eq!(retired.status(), StatusCode::ACCEPTED);
        let accepted: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(retired.into_body(), 1 << 20)
                .await
                .unwrap(),
        )
        .unwrap();
        let change = store
            .profile_change_get(accepted["changeId"].as_str().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(change.state, "Blocked");
        assert_eq!(change.reason.as_deref(), Some("ResourceInUse"));
        store.periodic_scan(1_800_000_003_000).await.unwrap();
        assert_eq!(get_json(&app, &uri).await["status"], "Retiring");
        let mut retry = authorized("DELETE", &uri, None);
        retry.headers_mut().insert("if-match", etag.clone());
        retry
            .headers_mut()
            .insert("idempotency-key", HeaderValue::from_static("delete-1"));
        let response = app.clone().oneshot(retry).await.unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let actual: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 1 << 20)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(actual, accepted);
    }
    assert!(store
        .auth_credential_bytes("prod-app", 1)
        .await
        .unwrap()
        .is_some());
    assert!(store
        .template_protected_bindings("k8s-linux", 1)
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn auth_resource_version_allows_strong_writes_but_never_weak_or_stale_ones() {
    let (app, store, _) = build_app_with_scan().await;
    let uri = "/api/v1/github-auth-profiles/prod-app";
    let created = app
        .clone()
        .oneshot(authorized("PUT", uri, Some(AUTH_PUT_BODY.into())))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::ACCEPTED);
    common::auth_fixture::promote(store.as_ref(), "prod-app", 1, 1)
        .await
        .unwrap();
    let mut view = app
        .clone()
        .oneshot(authorized("GET", uri, None))
        .await
        .unwrap();
    let strong = view.headers()["etag"].clone();
    let weak = HeaderValue::from_str(&format!("W/{}", strong.to_str().unwrap())).unwrap();
    // Model a compression proxy: ETag changes, the resource version does not.
    view.headers_mut().insert("etag", weak.clone());
    assert_eq!(view.headers().get("shaula-resource-version"), Some(&strong));
    let update = |version: HeaderValue| {
        let mut request = authorized(
            "PUT",
            uri,
            Some(AUTH_PUT_BODY.replace("github_app_test_key_bytes", "rotated-test-token")),
        );
        request.headers_mut().remove("if-none-match");
        request.headers_mut().insert("if-match", version);
        request
    };
    let rejected = app.clone().oneshot(update(weak)).await.unwrap();
    assert_eq!(rejected.status(), StatusCode::PRECONDITION_FAILED);
    assert_eq!(
        store
            .auth_profile_get("prod-app")
            .await
            .unwrap()
            .unwrap()
            .desired_revision,
        1
    );
    let accepted = app.clone().oneshot(update(strong.clone())).await.unwrap();
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    assert_ne!(
        accepted.headers().get("shaula-resource-version"),
        Some(&strong)
    );
    assert_eq!(
        accepted.headers().get("shaula-resource-version"),
        accepted.headers().get("etag")
    );
    let stale = app.clone().oneshot(update(strong)).await.unwrap();
    assert_eq!(stale.status(), StatusCode::PRECONDITION_FAILED);
    assert_eq!(
        store
            .auth_profile_get("prod-app")
            .await
            .unwrap()
            .unwrap()
            .desired_revision,
        2
    );
}

#[tokio::test]
async fn concurrent_auth_replacements_have_one_winner() {
    let (app, store, _) = build_app_with_scan().await;
    let uri = "/api/v1/github-auth-profiles/prod-app";
    let first = app
        .clone()
        .oneshot(authorized("PUT", uri, Some(AUTH_PUT_BODY.into())))
        .await
        .unwrap();
    let etag = first.headers()["etag"].clone();
    common::auth_fixture::promote(store.as_ref(), "prod-app", 1, 1)
        .await
        .unwrap();
    let update = |token: &str| {
        let mut req = authorized(
            "PUT",
            uri,
            Some(AUTH_PUT_BODY.replace("github_app_test_key_bytes", token)),
        );
        req.headers_mut().remove("if-none-match");
        req.headers_mut().insert("if-match", etag.clone());
        req
    };
    let (a, b) = tokio::join!(
        app.clone().oneshot(update("token-a")),
        app.clone().oneshot(update("token-b"))
    );
    let mut statuses = [a.unwrap().status().as_u16(), b.unwrap().status().as_u16()];
    statuses.sort();
    assert_eq!(statuses, [202, 412]);
    assert_eq!(
        store
            .auth_profile_get("prod-app")
            .await
            .unwrap()
            .unwrap()
            .desired_revision,
        2
    );
    assert!(store
        .auth_revision_get("prod-app", 3)
        .await
        .unwrap()
        .is_none());
}
