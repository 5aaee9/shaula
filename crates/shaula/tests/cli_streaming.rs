//! CLI behavior against a real HTTP listener with controlled paging faults.
use axum::{http::Uri, Json};
use serde_json::{json, Value};
use std::{
    process::Command,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
type TestResult = Result<(), Box<dyn std::error::Error>>;

#[tokio::test]
async fn follow_allows_idle_live_cursor_and_preserves_capture_metadata() -> TestResult {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let app = axum::Router::new().fallback(move |uri: Uri| {
        let counter = counter.clone();
        async move {
            if uri.path().ends_with("session") {
                return Json(json!({"name":"operator","scopes":["fleet.read","logs.read"]}));
            }
            let first = counter.fetch_add(1, Ordering::SeqCst) == 0;
            Json(json!({"invocation_id":"inv", "content_version":"one", "capture_status":"capturing",
                "has_gap":true,"lost_bytes":10,"next_cursor":"live-offset",
                "entries":if first { vec![json!({"command_ordinal":1,"sequence":1,"phase":"apply","stream":"stderr","observed_at":1,"text":"sanitized output"})] } else {vec![]}}))
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let output = tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_shaula"))
            .args([
                "--server",
                &origin,
                "--credential-kind",
                "oidc",
                "--allow-loopback-http",
                "logs",
                "read",
                "inv",
                "--follow",
                "--timeout",
                "2s",
            ])
            .env("SHAULA_ACCESS_TOKEN", "fixture-oidc")
            .env_remove("SHAULA_CONTEXT")
            .output()
    })
    .await??;
    task.abort();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let events: Vec<Value> = String::from_utf8(output.stdout)?
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    assert!(calls.load(Ordering::SeqCst) >= 2);
    assert!(!events.iter().any(|e| e["event"] == "error"));
    assert_eq!(events[0]["data"]["has_gap"], true);
    assert_eq!(events[0]["data"]["lost_bytes"], 10);
    assert_eq!(events.last().ok_or("end")?["event"], "end");
    assert_eq!(events.last().ok_or("end")?["metadata"]["partial"], true);
    Ok(())
}

#[tokio::test]
async fn all_invocations_preserves_latest_and_marks_cursor_cycles_partial() -> TestResult {
    let app = axum::Router::new().fallback(|uri: Uri| async move {
        if uri.path().ends_with("session") {
            Json(json!({"name":"operator","scopes":["fleet.read"]}))
        } else {
            Json(json!({"items":[{"id":"inv"}],"next_cursor":"cycle","latest_create":{"id":"newest"},"latest_destroy":null}))
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let output = tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_shaula"))
            .args([
                "--server",
                &origin,
                "--credential-kind",
                "oidc",
                "--allow-loopback-http",
                "invocations",
                "list",
                "generation",
                "--all",
            ])
            .env("SHAULA_ACCESS_TOKEN", "fixture-oidc")
            .env_remove("SHAULA_CONTEXT")
            .output()
    })
    .await??;
    task.abort();
    assert_eq!(output.status.code(), Some(9));
    let envelope: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(envelope["metadata"]["partial"], true);
    assert_eq!(envelope["data"]["latest_create"]["id"], "newest");
    assert!(envelope["data"]["items"]
        .as_array()
        .is_some_and(|items| !items.is_empty()));
    Ok(())
}
