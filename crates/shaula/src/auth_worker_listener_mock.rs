//! Real HTTP session/queue endpoints for composition regression tests.

use axum::{
    extract::Query,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{delete, get, patch, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::Notify;

type InventoryBarrier = (Arc<Notify>, Arc<Notify>);

#[derive(Default)]
pub(crate) struct ListenerMock {
    pub session_creates: AtomicUsize,
    pub session_deletes: AtomicUsize,
    pub polls: AtomicUsize,
    pub acks: AtomicUsize,
    pub acquisitions: AtomicUsize,
    pub emit_message: AtomicBool,
    pub expired: AtomicBool,
    pub expired_ack: AtomicBool,
    pub denied: AtomicBool,
    pub label_type: Mutex<Option<String>>,
    pub label_values: Mutex<Option<Value>>,
    pub label_updates: AtomicUsize,
    pub ignore_label_updates: AtomicBool,
    pub malformed_label_response: AtomicBool,
    pub unknown_runner: AtomicBool,
    pub inventory_barrier: Mutex<Option<InventoryBarrier>>,
}

impl ListenerMock {
    pub fn scale_set(&self) -> Value {
        json!({
            "id":42, "name":"shaula-x64", "runnerGroupId":7,
            "runnerGroupName":"Default",
            "labels": self.label_values.lock().unwrap().clone().unwrap_or_else(||
                json!([{"name":"shaula-x64", "type": self.label_type.lock().unwrap().as_deref().unwrap_or("system")}]))
        })
    }

    pub fn routes(self: &Arc<Self>, base: &str) -> Router {
        let sessions = self.clone();
        let deletes = self.clone();
        let polls = self.clone();
        let acks = self.clone();
        let acquisitions = self.clone();
        let labels = self.clone();
        let inventory = self.clone();
        let queue = format!("{base}/queue/f1");
        Router::new()
            .route("/actions/_apis/distributedtask/pools/0/agents", get(move || {
                let state = inventory.clone();
                async move {
                    let barrier = state.inventory_barrier.lock().unwrap().take();
                    if let Some((entered, release)) = barrier {
                        entered.notify_one();
                        release.notified().await;
                    }
                    Json(if state.unknown_runner.load(Ordering::SeqCst) {
                        json!({"count":1,"value":[{"id":101,"name":"foreign-runner","runnerScaleSetId":42}]})
                    } else { json!({"count":0,"value":[]}) })
                }
            }))
            .route("/actions/_apis/runtime/runnerscalesets/42", patch(move |Query(query): Query<HashMap<String,String>>, Json(body): Json<Value>| {
                let state = labels.clone();
                async move {
                    assert_eq!(query.get("api-version").map(String::as_str), Some("6.0-preview"));
                    assert_eq!(body.as_object().unwrap().len(), 1);
                    assert!(body["labels"].is_array());
                    state.label_updates.fetch_add(1, Ordering::SeqCst);
                    if !state.ignore_label_updates.load(Ordering::SeqCst) {
                        *state.label_values.lock().unwrap() = Some(body["labels"].clone());
                    }
                    if state.malformed_label_response.load(Ordering::SeqCst) {
                        (StatusCode::OK, "{").into_response()
                    } else { Json(state.scale_set()).into_response() }
                }
            }))
            .route("/actions/_apis/runtime/runnerscalesets/42/sessions", post(move || {
                let state = sessions.clone();
                let queue = queue.clone();
                async move {
                    let n = state.session_creates.fetch_add(1, Ordering::SeqCst) + 1;
                    Json(json!({
                        "sessionId":format!("6f9619ff-8b86-d011-b42d-{n:012}"),
                        "ownerName":"shaula",
                        "messageQueueUrl":queue,
                        "messageQueueAccessToken":"composition-queue-token",
                        "statistics":{"totalAssignedJobs":0}
                    }))
                }
            }))
            .route("/actions/_apis/runtime/runnerscalesets/42/sessions/{session}", delete(move || {
                let state = deletes.clone();
                async move { state.session_deletes.fetch_add(1, Ordering::SeqCst); StatusCode::NO_CONTENT }
            }))
            .route("/queue/f1", get(move |headers: HeaderMap, Query(query): Query<HashMap<String,String>>| {
                let state = polls.clone();
                async move {
                    assert_eq!(headers.get("authorization").unwrap(), "Bearer composition-queue-token");
                    assert_eq!(headers.get("x-scalesetmaxcapacity").unwrap(), "0");
                    if let Some(id) = query.get("lastMessageId") { assert_eq!(id, "1"); }
                    state.polls.fetch_add(1, Ordering::SeqCst);
                    if state.expired.load(Ordering::SeqCst) { return StatusCode::UNAUTHORIZED.into_response(); }
                    if state.denied.load(Ordering::SeqCst) { return StatusCode::FORBIDDEN.into_response(); }
                    if state.emit_message.swap(false, Ordering::SeqCst) {
                        Json(json!({
                            "messageId":1, "messageType":"RunnerScaleSetJobMessages",
                            "statistics":{"totalAssignedJobs":2},
                            "body":json!([{"messageType":"JobAvailable","runnerRequestId":11,"jobId":"job-11"}]).to_string()
                        })).into_response()
                    } else { StatusCode::ACCEPTED.into_response() }
                }
            }))
            .route("/queue/f1/1", delete(move |headers: HeaderMap| {
                let state = acks.clone();
                async move {
                    assert_eq!(headers.get("authorization").unwrap(), "Bearer composition-queue-token");
                    state.acks.fetch_add(1, Ordering::SeqCst);
                    if state.expired_ack.load(Ordering::SeqCst) {
                        return StatusCode::UNAUTHORIZED;
                    }
                    StatusCode::NO_CONTENT
                }
            }))
            .route("/actions/_apis/runtime/runnerscalesets/42/acquirejobs", post(move |headers: HeaderMap, Json(ids): Json<Vec<i64>>| {
                let state = acquisitions.clone();
                async move {
                    assert_eq!(headers.get("authorization").unwrap(), "Bearer composition-queue-token");
                    assert_eq!(ids, vec![11]);
                    state.acquisitions.fetch_add(1, Ordering::SeqCst);
                    Json(json!({"count":1,"value":[11]}))
                }
            }))
    }
}
