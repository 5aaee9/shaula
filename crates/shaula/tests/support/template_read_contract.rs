//! Full router/OIDC/SQLite contract regressions, specified before implementation.
//! Cover missing status, denied reads, desired-vs-historical revision projection,
//! schema-secret leakage and conservative fallback for unknown schema fields.
use crate::{common, template_updates};
use axum::http::{HeaderValue, StatusCode};
use serde_json::{json, Value};
use template_updates::{Fixture, TestResult, PROFILE};
use tower::ServiceExt;

async fn get(fixture: &Fixture, path: &str, scope: &str) -> TestResult<(StatusCode, Value)> {
    let mut request = common::authorized("GET", path, None);
    request.headers_mut().insert(
        "authorization",
        HeaderValue::from_str(&common::oidc::bearer(scope))?,
    );
    let response = fixture.app.clone().oneshot(request).await?;
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await?;
    Ok((
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
}

#[tokio::test]
async fn template_status_is_authorized_and_reports_validation() -> TestResult {
    let fixture = Fixture::new().await?;
    let path = format!("{PROFILE}/status");
    let (status, body) = get(&fixture, &path, "template.read").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["key"], "k8s-linux");
    assert_eq!(body["desiredRevision"], 1);
    assert_eq!(body["activeRevision"], 1);
    assert_eq!(body["status"], "Active");
    assert_eq!(body["validation"]["state"], "Active");
    assert_eq!(body["references"]["inUse"], true);
    assert_eq!(
        get(&fixture, &path, "template.publish").await?.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        get(
            &fixture,
            "/api/v1/template-profiles/missing/status",
            "template.read"
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    evidence("template-status", &body)?;
    Ok(())
}

#[tokio::test]
async fn all_template_reads_project_the_exact_revision_without_secrets() -> TestResult {
    let fixture = Fixture::new().await?;
    let schema = r#"{"type":"object","additionalProperties":false,"required":["namespace","kubeconfig"],"properties":{"namespace":{"type":"string","sensitive":false},"kubeconfig":{"type":"string","sensitive":true}}}"#;
    let (digest, bytes) = common::artifact_variants::with_bindings_schema(schema)?;
    assert_ne!(digest, fixture.base_digest);
    assert_ne!(fixture.body()["artifact_digest"], digest);
    fixture
        .store
        .store()
        .artifact_archive_put(&digest, &bytes, 1_800_000_001_000)
        .await?;
    let mut upload =
        common::authorized("PUT", &format!("/api/v1/template-artifacts/{digest}"), None);
    *upload.body_mut() = axum::body::Body::from(bytes);
    assert_eq!(
        fixture.app.clone().oneshot(upload).await?.status(),
        StatusCode::CREATED
    );
    let published = fixture.send(json!({"artifact_digest":digest,"engine_ref":"terraform","bindings":{"namespace":"public-namespace"}}), Some(&fixture.etag), None).await?;
    assert_eq!(published.0, StatusCode::ACCEPTED);
    let row = fixture.revision(2).await?;
    let body = common::attest_body(
        &fixture.store,
        &digest,
        row.bindings_digest.as_deref().ok_or("digest missing")?,
        &fixture.engine_binary,
    )
    .await;
    let attested =
        common::attestation_harness::put_attestation(&fixture.app, "k8s-linux", 2, "proof", body)
            .await;
    assert_eq!(attested.0, StatusCode::CREATED, "{}", attested.1);
    let expected =
        json!({"namespace":"public-namespace","kubeconfig":{"sensitive":true,"set":true}});
    let mut reports = Vec::new();
    for path in [
        PROFILE.to_string(),
        format!("{PROFILE}/status"),
        format!("{PROFILE}/revisions/2"),
        format!("{PROFILE}/revisions/2/attestations/proof"),
        "/api/v1/template-profiles".into(),
    ] {
        let (status, body) = get(&fixture, &path, "template.read").await?;
        assert_eq!(status, StatusCode::OK, "{path}");
        let bindings = if path == "/api/v1/template-profiles" {
            &body["profiles"][0]["bindings"]
        } else {
            &body["bindings"]
        };
        assert_eq!(bindings, &expected, "{path}");
        assert!(!body.to_string().contains("secret-kubeconfig"), "{path}");
        assert_eq!(
            get(&fixture, &path, "template.publish").await?.0,
            StatusCode::FORBIDDEN
        );
        reports.push(json!({"path":path,"body":body}));
    }
    // The older artifact has no field annotations; it must not borrow the
    // newly published artifact's public-field permission.
    let (_, old) = get(&fixture, &format!("{PROFILE}/revisions/1"), "template.read").await?;
    assert_eq!(
        old["bindings"]["namespace"],
        json!({"sensitive":true,"set":true})
    );
    evidence(
        "template-read-projections",
        &json!({"reads":reports,"historical":old}),
    )?;
    Ok(())
}

fn evidence(name: &str, value: &Value) -> TestResult {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/spec-contract-evidence");
    std::fs::create_dir_all(&root)?;
    std::fs::write(
        root.join(format!("{name}.json")),
        serde_json::to_vec_pretty(value)?,
    )?;
    Ok(())
}

#[tokio::test]
async fn template_status_cli_watch_uses_json_lines_against_real_router() -> TestResult {
    let fixture = Fixture::new().await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let app = fixture.app.clone();
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let bearer = common::oidc::bearer("template.read");
    let token = bearer.strip_prefix("Bearer ").ok_or("fixture bearer")?;
    let result = tokio::process::Command::new(env!("CARGO_BIN_EXE_shaula"))
        .args([
            "--server",
            &origin,
            "--allow-loopback-http",
            "--credential-kind",
            "oidc",
            "--output",
            "json",
            "templates",
            "status",
            "k8s-linux",
            "--watch",
            "--timeout",
            "1s",
        ])
        .env("SHAULA_ACCESS_TOKEN", token)
        .env_remove("SHAULA_CONTEXT")
        .output()
        .await?;
    let events = String::from_utf8(result.stdout)?
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()?;
    evidence(
        "template-status-cli-watch",
        &json!({"exit_code":result.status.code(),"events":events}),
    )?;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        events.first().ok_or("snapshot missing")?["event"],
        "snapshot"
    );
    assert_eq!(
        events.first().ok_or("snapshot missing")?["data"]["references"]["inUse"],
        true
    );
    assert_eq!(events.last().ok_or("end missing")?["event"], "end");
    server.abort();
    Ok(())
}
