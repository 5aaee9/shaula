//! Process-wide tracing, bounded metrics and OTLP/HTTP export.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use crate::traces::{Message, TraceLayer, TRACE_BUFFER};

mod traces;

static EXPORTER: OnceLock<Arc<OtlpExporter>> = OnceLock::new();
static TRACE_CONTROL: OnceLock<mpsc::Sender<Message>> = OnceLock::new();
static DEGRADED_EXPORTS: AtomicU64 = AtomicU64::new(0);

/// Keeps the telemetry pipeline open; dropping it signals the trace drain
/// to finish. Call [`TelemetryGuard::flush`] first for a bounded shutdown
/// flush (spec 0001 §12).
pub struct TelemetryGuard {
    trace_control: Option<mpsc::Sender<Message>>,
}

impl TelemetryGuard {
    /// Bounded shutdown flush: asks the trace pipeline to export everything
    /// buffered and gives up after `budget`. Without a pipeline this is a
    /// no-op.
    pub async fn flush(&self, budget: Duration) {
        let Some(control) = &self.trace_control else {
            return;
        };
        let (ack, ack_rx) = tokio::sync::oneshot::channel();
        if control.try_send(Message::Flush(ack)).is_err() {
            count_degraded_export(1);
            return;
        }
        if tokio::time::timeout(budget, ack_rx).await.is_err() {
            count_degraded_export(1);
        }
    }
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        if let Some(control) = &self.trace_control {
            let _ = control.try_send(Message::Shutdown);
        }
    }
}

/// Initializes local JSON logs and optional OTLP/HTTP export of metrics and
/// traces. Export failures are debug diagnostics and never block lifecycle
/// work.
pub fn init(service_name: &str, endpoint: Option<&str>) -> TelemetryGuard {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let collector = match endpoint
        .filter(|value| !value.trim().is_empty())
        .map(parse_collector_endpoint)
    {
        Some(Ok(collector)) => Some(collector),
        Some(Err(error)) => {
            tracing::warn!(summary = %error, "OTLP exporter unavailable; local telemetry remains enabled");
            None
        }
        None => None,
    };
    // Traces: the collecting layer must exist before the first span is
    // created, and the drain task needs a runtime. The first init wins —
    // the pipeline is a process-wide singleton.
    let mut trace_control = None;
    let mut trace_layer = None;
    if let Some(collector) = &collector {
        if let Ok(handle) = Handle::try_current() {
            let (tx, rx) = mpsc::channel(TRACE_BUFFER);
            if TRACE_CONTROL.set(tx.clone()).is_ok() {
                handle.spawn(traces::drain(
                    service_name.to_owned(),
                    collector.clone(),
                    rx,
                ));
                trace_layer = Some(TraceLayer::new(tx.clone()));
                trace_control = Some(tx);
            }
        } else {
            count_degraded_export(1);
        }
    }
    let metrics_exporter = collector.as_ref().map(|collector| OtlpExporter {
        endpoint: collector.clone(),
        service_name: service_name.to_owned(),
    });
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_current_span(true)
                .with_span_list(false)
                .flatten_event(true)
                .with_target(false),
        )
        .with(trace_layer)
        .try_init();
    if let Some(exporter) = metrics_exporter {
        let _ = EXPORTER.set(Arc::new(exporter));
    }
    tracing::info!(
        service = service_name,
        otlp = collector.is_some(),
        "telemetry initialized"
    );
    TelemetryGuard { trace_control }
}

pub fn init_local(service_name: &str) -> TelemetryGuard {
    init(service_name, None)
}

/// OTLP export failures observed since startup. This in-process counter is
/// the degraded-export signal (spec 0001 §13.2); local JSON telemetry stays
/// available regardless.
pub fn degraded_exports() -> u64 {
    DEGRADED_EXPORTS.load(Ordering::Relaxed)
}

fn count_degraded_export(delta: u64) {
    DEGRADED_EXPORTS.fetch_add(delta, Ordering::Relaxed);
}

/// Resolved collector location shared by both OTLP signals. Only plain
/// `http://` is accepted: TLS and authentication terminate at the collector
/// boundary, and staying off reqwest keeps this crate dependency-free
/// enough for `shaula-daemon` to depend on it (spec 0007 forbids reqwest in
/// the daemon's tree).
#[derive(Clone, Debug)]
pub struct CollectorEndpoint {
    host: String,
    port: u16,
    /// Host header value: `host`, or `host:port` for non-default ports.
    host_header: String,
    /// Collector base path: empty, or a `/prefixed` directory.
    base_path: String,
}

impl CollectorEndpoint {
    /// Request target for one OTLP signal: `metrics` or `traces`.
    pub(crate) fn signal_path(&self, signal: &str) -> String {
        format!("{}/v1/{signal}", self.base_path)
    }
}

/// Parses and validates an OTLP/HTTP collector base URL
/// (`http://host[:port][/base-path]`).
pub fn parse_collector_endpoint(endpoint: &str) -> Result<CollectorEndpoint, String> {
    let parsed =
        url::Url::parse(endpoint).map_err(|error| format!("invalid otlp endpoint: {error}"))?;
    if parsed.scheme() != "http" {
        return Err(format!(
            "otlp endpoint must be a plain http:// URL (TLS terminates at the collector boundary), got '{}'",
            parsed.scheme()
        ));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "otlp endpoint lacks a host".to_string())?
        .to_owned();
    let port = parsed
        .port_or_known_default()
        .ok_or_else(|| "otlp endpoint lacks a port".to_string())?;
    let host_header = if parsed.port().is_some() {
        format!("{host}:{port}")
    } else {
        host.clone()
    };
    Ok(CollectorEndpoint {
        host,
        port,
        host_header,
        base_path: parsed.path().trim_end_matches('/').to_owned(),
    })
}

const EXPORT_TIMEOUT: Duration = Duration::from_secs(5);

struct OtlpExporter {
    endpoint: CollectorEndpoint,
    service_name: String,
}

impl OtlpExporter {
    fn submit_metric(&self, operation: MetricOperation, result: MetricResult, delta: u64) {
        let payload = build_payload(&self.service_name, operation, result, delta);
        match Handle::try_current() {
            Ok(handle) => {
                let endpoint = self.endpoint.clone();
                let path = endpoint.signal_path("metrics");
                handle.spawn(post_payload(endpoint, path, payload.to_string(), 1));
            }
            // Outside a runtime there is no way to export; the event joins
            // the degraded counter instead of being silently dropped.
            Err(_) => count_degraded_export(1),
        }
    }
}

/// One bounded export attempt: a plain HTTP/1.1 POST, fire-and-forget.
/// Failures only bump the in-process degraded counter and log at debug —
/// a missing collector never fails lifecycle work.
async fn post_payload(endpoint: CollectorEndpoint, path: String, body: String, affected: u64) {
    let outcome =
        match tokio::time::timeout(EXPORT_TIMEOUT, exchange(&endpoint, &path, &body)).await {
            Ok(outcome) => outcome,
            Err(_) => Err(std::io::Error::other("export timed out")),
        };
    if let Err(error) = outcome {
        tracing::debug!(summary = %error, "OTLP export failed");
        count_degraded_export(affected);
    }
}

async fn exchange(endpoint: &CollectorEndpoint, path: &str, body: &str) -> std::io::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream =
        tokio::net::TcpStream::connect((endpoint.host.as_str(), endpoint.port)).await?;
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        endpoint.host_header,
        body.len()
    );
    stream.write_all(request.as_bytes()).await?;
    stream.flush().await?;
    // The response is not lifecycle-relevant, but a non-2xx means the
    // payload did not land, so the bounded drain classifies it. The
    // collector is expected to close (`Connection: close`).
    let mut response = Vec::new();
    let mut chunk = [0u8; 256];
    loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        response.extend_from_slice(&chunk[..read]);
        if response.len() > 4096 {
            break;
        }
    }
    let accepted = std::str::from_utf8(&response)
        .ok()
        .and_then(|text| text.lines().next())
        .and_then(|line| line.split_whitespace().nth(1))
        .is_some_and(|code| code.starts_with('2'));
    if accepted {
        Ok(())
    } else {
        Err(std::io::Error::other("collector did not accept the export"))
    }
}

/// Builds the single-metric OTLP/HTTP JSON body: one DELTA monotonic sum of
/// `shaula.operations.total` carrying only the finite `operation`/`result`
/// labels (spec 0001 §13.2 attribute allowlist).
fn build_payload(
    service_name: &str,
    operation: MetricOperation,
    result: MetricResult,
    delta: u64,
) -> Value {
    json!({
        "resourceMetrics": [{
            "resource": {"attributes": [
                {"key": "service.name", "value": {"stringValue": service_name}}
            ]},
            "scopeMetrics": [{
                "scope": {"name": "shaula"},
                "metrics": [{
                    "name": "shaula.operations.total",
                    "unit": "1",
                    "description": "Shaula operation outcomes",
                    "sum": {
                        "dataPoints": [{
                            "asInt": delta.to_string(),
                            "attributes": [
                                {"key": "operation", "value": {"stringValue": operation.as_str()}},
                                {"key": "result", "value": {"stringValue": result.as_str()}}
                            ]
                        }],
                        "aggregationTemporality": 1,
                        "isMonotonic": true
                    }
                }]
            }]
        }]
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricOperation {
    Http,
    Registry,
    Reconcile,
    Runner,
    IaC,
    Exporter,
}
impl MetricOperation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Registry => "registry",
            Self::Reconcile => "reconcile",
            Self::Runner => "runner",
            Self::IaC => "iac",
            Self::Exporter => "exporter",
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub enum MetricResult {
    Ok,
    Failed,
    Degraded,
}
impl MetricResult {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Failed => "failed",
            Self::Degraded => "degraded",
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TelemetryHandle;
impl TelemetryHandle {
    pub const fn new() -> Self {
        Self
    }
    /// Records one operation outcome: exported as a bounded OTLP delta point
    /// when an exporter is configured, and always mirrored to the local
    /// structured log. `MetricOperation::Exporter` never exports — it feeds
    /// the in-process degraded-export counter, so degradation reporting can
    /// never recurse into the failing exporter.
    pub fn record(&self, operation: MetricOperation, result: MetricResult, delta: u64) {
        if operation == MetricOperation::Exporter {
            count_degraded_export(delta);
        } else if let Some(exporter) = EXPORTER.get() {
            exporter.submit_metric(operation, result, delta);
        }
        tracing::debug!(
            operation = operation.as_str(),
            result = result.as_str(),
            delta,
            "telemetry metric"
        );
    }
}

#[cfg(test)]
pub(crate) async fn serve_one_http_exchange(
    listener: &tokio::net::TcpListener,
    response: &'static [u8],
) -> std::io::Result<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (mut socket, _) = listener.accept().await?;
    let mut buf = Vec::new();
    loop {
        let mut chunk = [0u8; 1024];
        let read = socket.read(&mut chunk).await?;
        buf.extend_from_slice(&chunk[..read]);
        let Some(header_end) = buf.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&buf[..header_end]).to_lowercase();
        let length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .map(|value| value.trim().parse::<usize>().unwrap_or(0))
            .unwrap_or(0);
        if buf.len() >= header_end + 4 + length {
            break;
        }
    }
    socket.write_all(response).await?;
    socket.shutdown().await?;
    Ok(String::from_utf8_lossy(&buf).to_string())
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "traces_tests.rs"]
mod traces_tests;
