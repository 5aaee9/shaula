//! Tests for the OTLP/HTTP trace export pipeline. Split from traces.rs to
//! keep both files within the 400-line limit (AGENTS.md).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use std::time::Duration;
use tracing_subscriber::layer::SubscriberExt;

use super::traces::{drain, Message, TraceLayer, TRACE_BUFFER};

fn install(layer: TraceLayer) -> tracing::subscriber::DefaultGuard {
    tracing::subscriber::set_default(tracing_subscriber::registry().with(layer))
}

#[tokio::test]
async fn spans_carry_correlated_ids_and_finite_attributes() {
    let (tx, mut rx) = mpsc::channel(TRACE_BUFFER);
    let _subscriber = install(TraceLayer::new(tx));
    let root = tracing::info_span!(
        "shaula.test.root",
        fleet_key = "fleet-1",
        attempts = 2u64,
        verbose = true
    );
    let child = tracing::info_span!(parent: &root, "shaula.test.child", note = "held");
    // The child closes first; both close without any runtime involvement.
    drop(child);
    drop(root);
    let Some(Message::Span(exported_child)) = rx.recv().await else {
        panic!("child span must be exported");
    };
    let Some(Message::Span(exported_root)) = rx.recv().await else {
        panic!("root span must be exported");
    };
    assert_eq!(exported_child["name"], "shaula.test.child");
    assert_eq!(exported_root["name"], "shaula.test.root");
    // The root is its own trace; the child inherits it and points at the
    // root's span id.
    assert_eq!(exported_child["traceId"], exported_root["traceId"]);
    assert_eq!(exported_child["parentSpanId"], exported_root["spanId"]);
    assert_eq!(exported_root["parentSpanId"], "");
    let root_attributes = exported_root["attributes"].to_string();
    let child_attributes = exported_child["attributes"].to_string();
    assert!(
        root_attributes.contains("fleet-1"),
        "root span must carry the fleet key attribute: {exported_root}"
    );
    assert!(
        root_attributes.contains("\"intValue\":\"2\""),
        "numeric fields export as OTLP int values: {exported_root}"
    );
    assert!(
        root_attributes.contains("\"boolValue\":true"),
        "boolean fields export as OTLP bool values: {exported_root}"
    );
    assert!(
        child_attributes.contains("held"),
        "child span must carry its own attribute: {exported_child}"
    );
}

#[tokio::test]
async fn foreign_targets_are_not_exported() {
    let (tx, mut rx) = mpsc::channel(TRACE_BUFFER);
    let _subscriber = install(TraceLayer::new(tx));
    let span = tracing::info_span!(target: "hyper::server", "foreign_span");
    drop(span);
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        message = rx.recv() => {
            panic!("a foreign-target span must not be exported: {message:?}")
        }
    }
}

#[tokio::test]
async fn drain_posts_batched_spans_to_the_traces_path() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        serve_one_http_exchange(
            &listener,
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await
    });
    let endpoint = parse_collector_endpoint(&format!("http://{addr}")).unwrap();
    let (tx, rx) = mpsc::channel(TRACE_BUFFER);
    let drain = tokio::spawn(drain("shaula-test".to_owned(), endpoint, rx));
    let _subscriber = install(TraceLayer::new(tx.clone()));
    let span = tracing::info_span!("shaula.test.exported", marker = "yes");
    drop(span);
    let (ack, ack_rx) = tokio::sync::oneshot::channel();
    tx.send(Message::Flush(ack)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), ack_rx)
        .await
        .unwrap()
        .unwrap();
    let request = server.await.unwrap().unwrap();
    drain.abort();
    assert!(
        request.starts_with("POST /v1/traces "),
        "unexpected request line: {request}"
    );
    assert!(
        request.contains("shaula.test.exported"),
        "the span name must reach the collector: {request}"
    );
    assert!(
        request.contains("\"stringValue\":\"yes\""),
        "span attributes must reach the collector: {request}"
    );
    assert!(
        request.contains("shaula-test"),
        "service.name must reach the collector: {request}"
    );
}
