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
        "/templates",
        "/auth",
        "/changes",
    ] {
        let response = request(path, Method::GET).await?;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "text/html");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-cache");
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
        "public, max-age=31536000, immutable"
    );
    let head = request(&format!("/{path}"), Method::HEAD).await?;
    assert_eq!(
        head.headers()[header::CONTENT_LENGTH],
        response.headers()[header::CONTENT_LENGTH]
    );
    assert!(to_bytes(head.into_body(), usize::MAX).await?.is_empty());
    Ok(())
}
