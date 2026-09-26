use axum::{http::StatusCode, response::Response, Router};
use tower::ServiceExt;

use crate::common;

pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub const POOL_URI: &str = "/api/v1/template-pools/builders";
pub const POOL_BODY: &str = r#"{
    "members": [{"key":"primary","template_profile_ref":"k8s-linux","weight":20,"template_inputs":{}}],
    "failure_policy":"backpressure"
}"#;

pub async fn put(
    app: &Router,
    uri: &str,
    body: &str,
    expected: Option<&str>,
    idem: &str,
) -> TestResult<Response> {
    let mut request = common::put_with_idempotency(uri, idem, body.into());
    if let Some(etag) = expected {
        request.headers_mut().remove("if-none-match");
        request.headers_mut().insert("if-match", etag.parse()?);
    }
    Ok(app.clone().oneshot(request).await?)
}

pub async fn accepted(
    response: Response,
    status: StatusCode,
) -> TestResult<(String, serde_json::Value)> {
    assert_eq!(response.status(), status);
    let etag = response
        .headers()
        .get("etag")
        .ok_or("missing ETag")?
        .to_str()?
        .to_string();
    assert_eq!(
        response.headers().get("shaula-resource-version"),
        response.headers().get("etag")
    );
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await?;
    Ok((etag, serde_json::from_slice(&bytes)?))
}
