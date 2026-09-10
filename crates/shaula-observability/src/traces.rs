//! OTLP/HTTP trace export: a `tracing` layer collects spans created under
//! shaula targets, batches them through a bounded channel and posts them to
//! the collector's `/v1/traces`. No OpenTelemetry SDK is linked: trace
//! identity derives from the registry's own 64-bit span ids (the root
//! span's id, zero-extended into the 16-byte `traceId`), which keeps
//! parent/child correlation consistent without extra dependencies.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tokio::sync::mpsc;
use tracing::span::{Attributes, Id, Record};
use tracing::Subscriber;
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

use super::{count_degraded_export, post_payload, CollectorEndpoint};

/// Bounded queue capacity: on overflow spans are dropped and counted as
/// degraded exports instead of ever blocking lifecycle work.
pub(crate) const TRACE_BUFFER: usize = 1024;
/// Batch size and interval driving the drain task's POSTs.
const TRACE_BATCH: usize = 64;
const TRACE_FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
/// Tracked-span cap so a pathological close-less leak cannot grow the map
/// without bound; further spans stay untracked (never exported).
const MAX_TRACKED_SPANS: usize = 4096;
const MAX_SPAN_ATTRIBUTES: usize = 16;

#[derive(Debug)]
pub(crate) enum Message {
    Span(Value),
    Flush(tokio::sync::oneshot::Sender<()>),
    Shutdown,
}

pub(crate) struct TraceLayer {
    tx: mpsc::Sender<Message>,
    spans: Mutex<HashMap<Id, SpanRecord>>,
}

impl TraceLayer {
    pub(crate) fn new(tx: mpsc::Sender<Message>) -> Self {
        Self {
            tx,
            spans: Mutex::new(HashMap::new()),
        }
    }

    fn lock_spans(&self) -> MutexGuard<'_, HashMap<Id, SpanRecord>> {
        // A panic while holding the lock must not permanently disable
        // trace collection.
        self.spans
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

struct SpanRecord {
    name: &'static str,
    start_unix_nano: u128,
    parent: Option<u64>,
    trace_id: u64,
    attributes: Vec<(String, Value)>,
}

impl<S> Layer<S> for TraceLayer
where
    S: Subscriber,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let metadata = attrs.metadata();
        if !metadata.target().starts_with("shaula") {
            return;
        }
        let parent = if attrs.is_contextual() {
            ctx.current_span().id().cloned()
        } else {
            attrs.parent().cloned()
        };
        let trace_id = parent
            .as_ref()
            .and_then(|parent| self.lock_spans().get(parent).map(|record| record.trace_id));
        let mut attributes = Vec::new();
        attrs.record(&mut AttributeVisitor(&mut attributes));
        let mut spans = self.lock_spans();
        if spans.len() >= MAX_TRACKED_SPANS {
            return;
        }
        let span_id = id.into_u64();
        spans.insert(
            id.clone(),
            SpanRecord {
                name: metadata.name(),
                start_unix_nano: now_unix_nano(),
                parent: parent.map(|parent| parent.into_u64()),
                trace_id: trace_id.unwrap_or(span_id),
                attributes,
            },
        );
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, _ctx: Context<'_, S>) {
        // Only tracked spans are in the map, and only shaula spans are
        // tracked — map membership IS the target filter here.
        let mut spans = self.lock_spans();
        let Some(record) = spans.get_mut(id) else {
            return;
        };
        values.record(&mut AttributeVisitor(&mut record.attributes));
    }

    fn on_close(&self, id: Id, _ctx: Context<'_, S>) {
        let record = self.lock_spans().remove(&id);
        let Some(record) = record else {
            return;
        };
        let span = json!({
            "traceId": format!("{:032x}", record.trace_id),
            "spanId": format!("{:016x}", id.into_u64()),
            "parentSpanId": record
                .parent
                .map(|parent| format!("{parent:016x}"))
                .unwrap_or_default(),
            "name": record.name,
            "startTimeUnixNano": record.start_unix_nano.to_string(),
            "endTimeUnixNano": now_unix_nano().to_string(),
            "attributes": record
                .attributes
                .iter()
                .map(|(key, value)| json!({"key": key, "value": value}))
                .collect::<Vec<_>>(),
        });
        if self.tx.try_send(Message::Span(span)).is_err() {
            count_degraded_export(1);
        }
    }
}

struct AttributeVisitor<'a>(&'a mut Vec<(String, Value)>);

impl AttributeVisitor<'_> {
    fn push(&mut self, name: &str, value: Value) {
        if self.0.len() < MAX_SPAN_ATTRIBUTES && !name.starts_with("log.") {
            self.0.push((name.to_owned(), value));
        }
    }
}

impl tracing::field::Visit for AttributeVisitor<'_> {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.push(field.name(), json!({"stringValue": value}));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.push(field.name(), json!({"boolValue": value}));
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.push(field.name(), json!({"intValue": value.to_string()}));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.push(field.name(), json!({"intValue": value.to_string()}));
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        self.push(field.name(), json!({"stringValue": format!("{value:?}")}));
    }
}

/// Drains the span channel until shutdown: posts batches when full, on the
/// periodic tick, on an explicit flush or once at the end. A missing
/// collector only produces degraded-export counts.
pub(crate) async fn drain(
    service_name: String,
    endpoint: CollectorEndpoint,
    mut rx: mpsc::Receiver<Message>,
) {
    let path = endpoint.signal_path("traces");
    let mut ticker = tokio::time::interval(TRACE_FLUSH_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut batch: Vec<Value> = Vec::with_capacity(TRACE_BATCH);
    loop {
        let mut finishing = false;
        tokio::select! {
            message = rx.recv() => match message {
                Some(Message::Span(span)) => {
                    batch.push(span);
                    if batch.len() >= TRACE_BATCH {
                        flush_batch(&endpoint, &path, &service_name, &mut batch).await;
                    }
                }
                Some(Message::Flush(ack)) => {
                    flush_batch(&endpoint, &path, &service_name, &mut batch).await;
                    let _ = ack.send(());
                }
                Some(Message::Shutdown) | None => finishing = true,
            },
            _ = ticker.tick() => {
                flush_batch(&endpoint, &path, &service_name, &mut batch).await;
            }
        }
        if finishing {
            flush_batch(&endpoint, &path, &service_name, &mut batch).await;
            return;
        }
    }
}

async fn flush_batch(
    endpoint: &CollectorEndpoint,
    path: &str,
    service_name: &str,
    batch: &mut Vec<Value>,
) {
    if batch.is_empty() {
        return;
    }
    let spans = std::mem::take(batch);
    let payload = json!({
        "resourceSpans": [{
            "resource": {"attributes": [
                {"key": "service.name", "value": {"stringValue": service_name}}
            ]},
            "scopeSpans": [{
                "scope": {"name": "shaula"},
                "spans": spans,
            }],
        }]
    });
    post_payload(
        endpoint.clone(),
        path.to_owned(),
        payload.to_string(),
        spans.len() as u64,
    )
    .await;
}

fn now_unix_nano() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}
