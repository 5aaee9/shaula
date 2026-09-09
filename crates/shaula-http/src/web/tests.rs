use super::*;
use axum::{body::to_bytes, http::Request, Router};
use tower::ServiceExt;

type TestResult = Result<(), Box<dyn std::error::Error>>;

async fn request(path: &str, method: Method) -> Result<Response, Box<dyn std::error::Error>> {
    Ok(Router::new()
        .fallback(serve)
        .oneshot(
            Request::builder()
                .uri(path)
                .method(method)
                .body(Body::empty())?,
        )
        .await?)
}

#[tokio::test]
async fn web_tests_embedded_document_and_deep_link() -> TestResult {
    for path in [
        "/",
        "/fleets",
        "/fleets/linux-build",
        "/fleets/linux-x64.1_a",
        "/templates",
        "/templates/new",
        "/templates/linux-build/revisions/new",
        "/templates/linux-x64.1_a/revisions/new",
        "/templates/linux-build/update",
        "/templates/linux-x64.1_a/update",
        "/auth",
        "/auth/new",
        "/auth/shared-github/targets/edit",
        "/auth/shared-github/rotate",
        "/changes",
        "/jobs",
        "/jobs/job-1",
        "/jobs/runners",
        "/jobs/runners/00000000-0000-0000-0000-000000000001",
    ] {
        let response = request(path, Method::GET).await?;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "text/html");
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "private, no-store"
        );
        assert!(response
            .headers()
            .contains_key(header::CONTENT_SECURITY_POLICY));
        let body = to_bytes(response.into_body(), usize::MAX).await?;
        assert!(std::str::from_utf8(&body)?.contains("<div id=\"root\">"));
    }
    Ok(())
}

#[tokio::test]
async fn web_tests_missing_assets_api_and_methods_are_not_html() -> TestResult {
    for path in [
        "/api",
        "/api/v1/missing",
        "/assets/missing.js",
        "/missing.css",
        "/unknown",
        "/jobs//runner",
        "/jobs/job-1/extra",
        "/jobs/runners/../fleet",
        "/jobs/%2fexternal",
        "/templates/linux-build",
        "/templates/new/extra",
        "/templates//revisions/new",
        "/templates/../revisions/new",
        "/templates/%2e%2e/revisions/new",
        "/templates/with%20space/revisions/new",
        "/templates/a/b/revisions/new",
        "/templates/linux-build/revisions/latest",
        "/templates/linux-build/revisions/new/",
        "/templates//update",
        "/templates/../update",
        "/templates/%2e%2e/update",
        "/templates/a/b/update",
        "/templates/linux-build/update/",
        "/auth/shared-github",
        "/auth/new/extra",
        "/auth//targets/edit",
        "/auth/../targets/edit",
        "/auth/%2e%2e/targets/edit",
        "/auth/a/b/rotate",
        "/auth/shared-github/rotate/",
    ] {
        assert_eq!(
            request(path, Method::GET).await?.status(),
            StatusCode::NOT_FOUND
        );
    }
    let response = request("/fleets", Method::POST).await?;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(response.headers()[header::ALLOW], "GET, HEAD");
    Ok(())
}

#[tokio::test]
async fn web_tests_template_publication_head_and_post() -> TestResult {
    for path in [
        "/templates/new",
        "/templates/linux-build/revisions/new",
        "/templates/linux-build/update",
        "/auth/new",
        "/auth/shared-github/targets/edit",
        "/auth/shared-github/rotate",
    ] {
        let response = request(path, Method::HEAD).await?;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "text/html");
        assert!(to_bytes(response.into_body(), usize::MAX).await?.is_empty());

        let response = request(path, Method::POST).await?;
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(response.headers()[header::ALLOW], "GET, HEAD");
    }
    Ok(())
}

#[tokio::test]
async fn web_tests_assets_and_head_have_correct_headers() -> TestResult {
    let path = WebAssets::iter()
        .find(|path| path.ends_with(".js"))
        .ok_or("missing bundled JS")?;
    let response = request(&format!("/{path}"), Method::GET).await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers()[header::CONTENT_TYPE]
        .to_str()?
        .contains("javascript"));
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );
    let head = request(&format!("/{path}"), Method::HEAD).await?;
    assert_eq!(
        head.headers()[header::CONTENT_LENGTH],
        response.headers()[header::CONTENT_LENGTH]
    );
    assert!(to_bytes(head.into_body(), usize::MAX).await?.is_empty());
    Ok(())
}
