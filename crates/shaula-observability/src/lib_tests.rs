//! Tests for the metrics facade and OTLP/HTTP export. Split from lib.rs to
//! stay within the 400-line limit (AGENTS.md).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use std::time::{Duration, Instant};

#[test]
fn payload_carries_finite_labels_and_delta_temporality() {
    let body = build_payload(
        "shaula-test",
        MetricOperation::Registry,
        MetricResult::Degraded,
        3,
    );
    let resource = &body["resourceMetrics"][0]["resource"]["attributes"][0];
    assert_eq!(resource["key"], "service.name");
    assert_eq!(resource["value"]["stringValue"], "shaula-test");
    let sum = &body["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["sum"];
    assert_eq!(sum["aggregationTemporality"], 1);
    assert_eq!(sum["isMonotonic"], true);
    let point = &sum["dataPoints"][0];
    assert_eq!(point["asInt"], "3");
    assert_eq!(point["attributes"][0]["key"], "operation");
    assert_eq!(point["attributes"][0]["value"]["stringValue"], "registry");
    assert_eq!(point["attributes"][1]["key"], "result");
    assert_eq!(point["attributes"][1]["value"]["stringValue"], "degraded");
}

#[test]
fn exporter_operation_counts_degraded_without_exporting() {
    let before = degraded_exports();
    TelemetryHandle::new().record(MetricOperation::Exporter, MetricResult::Degraded, 2);
    assert_eq!(degraded_exports(), before + 2);
}

#[test]
fn endpoint_parser_joins_signal_paths_and_rejects_tls() {
    let endpoint = parse_collector_endpoint("http://collector:4318/otel/").unwrap();
    assert_eq!(endpoint.host, "collector");
    assert_eq!(endpoint.port, 4318);
    assert_eq!(endpoint.host_header, "collector:4318");
    assert_eq!(endpoint.signal_path("metrics"), "/otel/v1/metrics");
    assert_eq!(endpoint.signal_path("traces"), "/otel/v1/traces");
    let bare = parse_collector_endpoint("http://collector").unwrap();
    assert_eq!(bare.port, 80);
    assert_eq!(bare.host_header, "collector");
    assert_eq!(bare.signal_path("traces"), "/v1/traces");
    let error = parse_collector_endpoint("https://collector:4318").unwrap_err();
    assert!(
        error.contains("http://"),
        "TLS belongs to the collector boundary, not this exporter: {error}"
    );
}

#[tokio::test]
async fn init_exports_metrics_to_the_metrics_path() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        serve_one_http_exchange(
            &listener,
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await
    });
    let _telemetry = init("shaula-test", Some(&format!("http://{addr}")));
    TelemetryHandle::new().record(MetricOperation::IaC, MetricResult::Failed, 1);
    let request = server.await.unwrap().unwrap();
    assert!(
        request.starts_with("POST /v1/metrics "),
        "unexpected request line: {request}"
    );
    assert!(
        request.contains("\"stringValue\":\"failed\""),
        "missing result label: {request}"
    );
    assert!(
        request.contains("shaula-test"),
        "missing service.name: {request}"
    );
}

#[tokio::test]
async fn a_non_2xx_collector_response_counts_as_degraded() {
    // A collector that answers but rejects the payload must surface in the
    // degraded counter, not silently succeed.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        serve_one_http_exchange(
            &listener,
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await
    });
    let _telemetry = init("shaula-test", Some(&format!("http://{addr}")));
    let before = degraded_exports();
    TelemetryHandle::new().record(MetricOperation::Http, MetricResult::Ok, 1);
    let _ = server.await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while degraded_exports() == before {
        assert!(
            Instant::now() < deadline,
            "the rejected export never reached the degraded counter"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[test]
fn local_init_is_idempotent() {
    let _first = init_local("shaula-test");
    let _second = init_local("shaula-test-again");
    TelemetryHandle::new().record(MetricOperation::Http, MetricResult::Ok, 1);
}
