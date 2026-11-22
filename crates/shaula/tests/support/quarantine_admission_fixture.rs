use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use shaula_core::lifecycle::GenerationState as G;
use shaula_core::ports::ledger_metrics::WorkerAdmissionLimits;
use shaula_core::registry::GenerationRecord;
use shaula_core::state_backend::{LockInfo, StateAccess, StateBackend};
use shaula_core::worker::{ControlAccess, FenceOutcome, ProcessIdentity};
use shaula_daemon::service::ControlPlane;
use shaula_store::http_state::{SqliteStateBackend, WorkerAdmissions};
use shaula_store::Store;
use uuid::Uuid;

use super::common::oidc;

pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn sql(query: &str, values: Vec<sea_orm::Value>) -> sea_orm::Statement {
    sea_orm::Statement::from_sql_and_values(sea_orm::DbBackend::Sqlite, query, values)
}

pub fn finalize_req(id: &str, reason: &str) -> TestResult<Request<Body>> {
    Ok(Request::builder()
        .method("POST")
        .uri(format!("/api/v1/generations/{id}/finalize"))
        .header("authorization", oidc::bearer("fleet.retire"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "reason": reason }).to_string(),
        ))?)
}

pub fn gen_record(id: &str, digest: &str, now: i64) -> GenerationRecord {
    GenerationRecord {
        id: id.into(),
        fleet_key: "f1".into(),
        runner_name: format!("runner-{id}"),
        generation_name: format!("gen-{id}"),
        fleet_revision: 1,
        pool_member_key: None,
        template_profile_key: "k8s-linux".into(),
        template_revision: 1,
        template_artifact_digest: digest.into(),
        attestation_id: "att".into(),
        inputs_digest: "inputs".into(),
        state: G::CreatePending,
        github_runner_id: None,
        workspace_path: "unused".into(),
        created_at: now,
        updated_at: now,
    }
}

pub async fn response_json(response: axum::response::Response) -> TestResult<(StatusCode, Value)> {
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await?;
    Ok((status, serde_json::from_slice(&bytes)?))
}

pub async fn scrape_metrics(
    store: &Store,
    service: Arc<ControlPlane>,
    limits: WorkerAdmissionLimits,
) -> TestResult<String> {
    let scrape = Arc::new(shaula_http::metrics::Scrape {
        ledger: Arc::new(store.clone()),
        fleets: service.clone(),
        health: service,
        started_at: 1_800_000_000.0,
        worker_limits: Some(limits),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let server = tokio::spawn(shaula_http::server::serve_bound(
        shaula_http::metrics::router(scrape),
        listener,
        stopped,
    ));
    let body = reqwest::Client::builder()
        .no_proxy()
        .build()?
        .get(format!("http://{addr}/metrics"))
        .send()
        .await?
        .text()
        .await?;
    stop.send(true)?;
    server.await??;
    Ok(body)
}

pub async fn quarantine_with_stale_lock(
    backend: &SqliteStateBackend,
    admissions: &WorkerAdmissions,
    generation_id: Uuid,
    outcome: FenceOutcome,
) -> TestResult {
    let admission = admissions.take(generation_id)?;
    let state = StateAccess {
        generation_id,
        capability: admission.state,
    };
    let control = ControlAccess {
        claim: admission.claim.clone(),
        capability: admission.control,
    };
    backend.worker_launch_pending(&control).await?;
    backend
        .worker_register(
            &control,
            &ProcessIdentity {
                host_boot: "host:boot".into(),
                process_id: 4242,
                started: "100".into(),
                containment: format!("/cgroup/{generation_id}"),
                root_id: None,
            },
        )
        .await?;
    backend.note_create_starting(&admission.claim).await?;
    let lock = LockInfo::parse(
        serde_json::json!({
            "ID": Uuid::new_v4().to_string(),
            "Operation": "OperationTypeApply",
            "Who": "@host",
            "Version": "1.9.8",
            "Created": "2026-11-22T00:00:00Z",
            "Path": ""
        })
        .to_string()
        .as_bytes(),
    )?;
    backend.lock(&state, lock).await?;
    let record = backend
        .recovery_records()
        .await?
        .into_iter()
        .find(|row| row.generation_id == generation_id.to_string())
        .ok_or("recovery record missing")?;
    backend
        .recover_worker(&record, outcome, admissions, false)
        .await?;
    Ok(())
}

pub fn assert_lines(body: &str, expected: &[&str]) {
    for line in expected {
        assert!(
            body.lines().any(|actual| actual == *line),
            "missing `{line}` in scrape:\n{body}"
        );
    }
}
