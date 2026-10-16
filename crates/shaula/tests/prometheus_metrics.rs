//! E2E for the Prometheus surface: the real supervisor drives a Create, a
//! failed-then-retried Runner Resource destroy and a quarantined
//! Retirement against the fake runtime, then a real HTTP client scrapes
//! the loopback `/metrics` listener. The scraped body is written to
//! `$CARGO_TARGET_TMPDIR/prometheus-metrics/scrape.prom`.
//!
//! One test per binary: the counters are process-wide, so the expected
//! values below are exact.
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod common;
#[allow(clippy::unwrap_used)]
#[path = "common/lifecycle_fakes.rs"]
mod fakes;
#[path = "support/runner_lifetime_fixture.rs"]
mod fixture;

use fixture::{Fixture, TestResult};
use shaula_core::{lifecycle::GenerationState as G, registry::LifecycleStore};
use shaula_daemon::service::ControlPlane;
use std::{sync::atomic::Ordering, sync::Arc, time::Duration};

/// One successful Create through the real supervisor.
async fn create() -> TestResult {
    let f = Fixture::new().await?;
    f.github.jit_definite.store(true, Ordering::SeqCst);
    f.store.demand_snapshot("f1", 1, 1000).await?;
    let supervisor = f.supervisor(Duration::from_secs(7200))?;
    supervisor.tick(1000).await?; // settle handoff
    assert_eq!(supervisor.tick(1000).await?.created, 1);
    Ok(())
}

/// A Retirement whose Create left no ownership proof is quarantined.
async fn quarantine() -> TestResult {
    let f = Fixture::new().await?;
    f.seed(false).await?;
    let supervisor = f
        .supervisor(Duration::from_secs(10))?
        .with_execution_ready(false);
    supervisor.tick(11_000).await?;
    assert_eq!(f.state().await?, G::Quarantined);
    Ok(())
}

async fn scrape(f: &Fixture) -> TestResult<(String, String)> {
    let control = Arc::new(ControlPlane::new(
        f.store.clone(),
        f.clock.clone(),
        b"metrics-e2e-bindings-key".to_vec(),
        100,
        std::path::PathBuf::from("target/unused-engine"),
    ));
    control.set_ready(true);
    let scrape = Arc::new(shaula_http::metrics::Scrape {
        ledger: Arc::new(f.store.store().clone()),
        fleets: control.clone(),
        health: control,
        started_at: 1_800_000_000.0,
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let server = tokio::spawn(shaula_http::server::serve_bound(
        shaula_http::metrics::router(scrape),
        listener,
        stopped,
    ));
    let client = reqwest::Client::builder().no_proxy().build()?;
    let response = client
        .get(format!("http://{address}/metrics"))
        .send()
        .await?;
    assert_eq!(response.status(), 200);
    let content_type = response
        .headers()
        .get("content-type")
        .ok_or("content type missing")?
        .to_str()?
        .to_owned();
    let body = response.text().await?;
    let other = client
        .get(format!("http://{address}/api/v1/fleets"))
        .send()
        .await?;
    assert_eq!(other.status(), 404, "only /metrics is served");
    stop.send(true)?;
    server.await??;
    Ok((content_type, body))
}

#[tokio::test]
async fn scrape_reports_lifecycle_effects_with_bounded_labels() -> TestResult {
    create().await?;
    quarantine().await?;

    // Retirement whose first destroy fails and whose retry succeeds.
    let f = Fixture::new().await?;
    f.seed(true).await?;
    f.runtime.fail.store(true, Ordering::SeqCst);
    let supervisor = f
        .supervisor(Duration::from_secs(10))?
        .with_execution_ready(false);
    supervisor.tick(11_000).await?;
    assert_eq!(f.state().await?, G::DestroyPending);
    f.runtime.fail.store(false, Ordering::SeqCst);
    assert_eq!(supervisor.tick(26_000).await?.destroyed, 1);
    assert_eq!(f.state().await?, G::Destroyed);
    f.store
        .store()
        .execute_for_tests(
            "INSERT INTO workflow_jobs(id,scope_key,fleet_key,fleet_incarnation,scale_set_id,
                protocol_job_id,summary_json,status,created_at,updated_at)
             VALUES('job-secret-id','scope','f1','inc',42,'protocol',
                '{\"reported_result\":\"Success\"}','completed',1,1)",
        )
        .await?;

    let (content_type, body) = scrape(&f).await?;
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("prometheus-metrics");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("scrape.prom"), &body)?;

    assert_eq!(content_type, shaula_http::metrics::CONTENT_TYPE);
    for expected in [
        "# TYPE shaula_up gauge",
        "shaula_up 1",
        "shaula_ready 1",
        &format!(
            "shaula_build_info{{version=\"{}\"}} 1",
            env!("CARGO_PKG_VERSION")
        ),
        "shaula_generations{fleet=\"f1\",state=\"Destroyed\"} 1",
        "shaula_jobs{fleet=\"f1\",backend=\"github\",status=\"completed\",result=\"success\"} 1",
        "shaula_fleet_capacity_occupancy{fleet=\"f1\"} 0",
        "shaula_fleet_capacity_target{fleet=\"f1\"} 0",
        "shaula_fleet_assigned_demand{fleet=\"f1\"} 0",
        "# TYPE shaula_iac_operations_total counter",
        "shaula_iac_operations_total{operation=\"create\",result=\"ok\"} 1",
        "shaula_iac_operations_total{operation=\"destroy\",result=\"execution_failed\"} 1",
        "shaula_iac_operations_total{operation=\"destroy\",result=\"ok\"} 1",
        "shaula_iac_failures_total{operation=\"destroy\",result=\"execution_failed\",phase=\"injected\"} 1",
        "# TYPE shaula_iac_operation_duration_seconds histogram",
        "shaula_iac_operation_duration_seconds_count{operation=\"destroy\",result=\"ok\"} 1",
        "shaula_iac_operation_duration_seconds_bucket{operation=\"create\",result=\"ok\",le=\"+Inf\"} 1",
        "shaula_destroy_attempts_total{attempt=\"first\"} 1",
        "shaula_destroy_attempts_total{attempt=\"retry\"} 1",
        "shaula_destroy_quarantines_total{reason=\"destroy_proof_missing\"} 1",
        "shaula_operations_total{operation=\"iac\",result=\"ok\"} 2",
    ] {
        assert!(
            body.lines().any(|line| line == expected),
            "missing `{expected}` in scrape:\n{body}"
        );
    }
    // Bounded labels: no Generation/job identity or scope ever appears.
    for leaked in ["\"gen\"", "job-secret-id", "protocol", "scope"] {
        assert!(!body.contains(leaked), "{leaked} leaked into:\n{body}");
    }
    Ok(())
}
