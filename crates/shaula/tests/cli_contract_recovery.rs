//! Real CLI + HTTP fault fixtures. Failures covered before implementation:
//! accepted/lost attestation ACK must retain its exact retry key; Ctrl-C after
//! acceptance must retain the receipt and use cancellation exit code 130.
use axum::{
    extract::State,
    http::HeaderMap,
    routing::{get, put},
    Json,
};
use serde_json::{json, Value};
use std::{
    process::Stdio,
    sync::{Arc, Mutex},
};
type TestResult = Result<(), Box<dyn std::error::Error>>;

fn artifact(name: &str, value: &Value) -> TestResult {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/spec-contract-evidence");
    std::fs::create_dir_all(&root)?;
    std::fs::write(
        root.join(format!("{name}.json")),
        serde_json::to_vec_pretty(value)?,
    )?;
    Ok(())
}

fn command(origin: &str) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_shaula"));
    command
        .args([
            "--server",
            origin,
            "--credential-kind",
            "oidc",
            "--allow-loopback-http",
            "--output",
            "json",
        ])
        .env("SHAULA_ACCESS_TOKEN", "fixture-oidc")
        .env_remove("SHAULA_CONTEXT")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}
async fn session() -> Json<Value> {
    Json(json!({"name":"fixture", "scopes":["fleet.read","fleet.write","template.attest"]}))
}

#[tokio::test]
async fn attestation_ack_loss_retains_key_for_same_attempt_replay() -> TestResult {
    let keys = Arc::new(Mutex::new(Vec::<String>::new()));
    let app = axum::Router::new()
        .route("/api/v1/session", get(session))
        .route(
            "/api/v1/template-profiles/review/revisions/1/attestations/proof",
            put(
                |State(keys): State<Arc<Mutex<Vec<String>>>>, headers: HeaderMap| async move {
                    let key = headers["idempotency-key"]
                        .to_str()
                        .unwrap_or_default()
                        .to_owned();
                    let first = {
                        let mut keys = keys.lock().unwrap_or_else(|e| e.into_inner());
                        keys.push(key);
                        keys.len() == 1
                    };
                    // An accepted response with an interrupted body represents lost ACK,
                    // not a definite rejection. The replay returns the original result.
                    let body = if first {
                        "{".to_owned()
                    } else {
                        json!({"attestationId":"proof"}).to_string()
                    };
                    (
                        axum::http::StatusCode::CREATED,
                        [("content-type", "application/json")],
                        body,
                    )
                },
            ),
        )
        .with_state(keys.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let dir = tempfile::tempdir()?;
    let body = dir.path().join("body.json");
    std::fs::write(&body, "{}")?;
    let args = [
        "templates",
        "attestations",
        "create",
        "review",
        "1",
        "proof",
        "--file",
    ];
    let first = command(&origin).args(args).arg(&body).output().await?;
    let lost: Value = serde_json::from_slice(&first.stdout)?;
    artifact("attestation-lost-ack", &lost)?;
    assert_eq!(first.status.code(), Some(9));
    assert_eq!(lost["receipt"]["outcome"], "uncertain");
    let key = lost["receipt"]["idempotency_key"]
        .as_str()
        .ok_or("missing retry key")?;
    let replay = command(&origin)
        .args(args)
        .arg(&body)
        .args(["--idempotency-key", key])
        .output()
        .await?;
    let recovered: Value = serde_json::from_slice(&replay.stdout)?;
    artifact("attestation-replayed", &recovered)?;
    assert!(replay.status.success());
    assert_eq!(recovered["receipt"]["idempotency_key"], key);
    assert_eq!(recovered["receipt"]["outcome"], "completed");
    assert_eq!(*keys.lock().map_err(|_| "poisoned")?, [key, key]);
    server.abort();
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn cancellation_after_acceptance_keeps_receipt_and_exits_130() -> TestResult {
    let waiting = Arc::new(tokio::sync::Notify::new());
    let reached = waiting.clone();
    let app = axum::Router::new().route("/api/v1/session", get(session))
        .route("/api/v1/fleets/review", put(|| async { Json(json!({"changeId":"change-1","state":"Accepted","revision":1})) }))
        .route("/api/v1/fleet-changes/change-1", get(move || { let reached = reached.clone(); async move {
            reached.notify_one();
            Json(json!({"id":"change-1","resource_kind":"fleet","resource_key":"review","revision":1,"kind":"Update","state":"Pending","reason":null}))
        }}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let dir = tempfile::tempdir()?;
    let body = dir.path().join("body.json");
    std::fs::write(&body, "{}")?;
    let child = command(&origin)
        .args(["fleets", "create", "review", "--wait", "--file"])
        .arg(body)
        .spawn()?;
    tokio::time::timeout(std::time::Duration::from_secs(10), waiting.notified()).await?;
    let sent = tokio::process::Command::new("kill")
        .args(["-INT", &child.id().ok_or("child exited")?.to_string()])
        .status()
        .await?;
    assert!(sent.success());
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), child.wait_with_output())
        .await??;
    let output: Value = serde_json::from_slice(&result.stdout)?;
    artifact(
        "cancel-accepted-wait",
        &json!({"exit_code":result.status.code(),"output":output}),
    )?;
    assert_eq!(result.status.code(), Some(130));
    assert_eq!(output["error"]["code"], 130);
    assert_eq!(output["receipt"]["outcome"], "accepted");
    assert_eq!(output["receipt"]["change"]["id"], "change-1");
    server.abort();
    Ok(())
}
