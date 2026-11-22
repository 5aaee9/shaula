//! E2E verification for issue #7 (operator finalize on quarantined generations
//! holding a stale state lock and non-misleading error classification) and
//! issue #8 (worker admission limit reported as `AdmissionLimitReached` instead
//! of `StorageUnavailable`, diagnostic reason capture, and Prometheus worker
//! headroom metrics).
//!
//! Writes verifiable artifacts under
//! `$CARGO_TARGET_TMPDIR/quarantine-finalize-admission/`.

#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod common;
#[path = "support/quarantine_admission_fixture.rs"]
mod fixture;

use std::sync::Arc;

use axum::http::StatusCode;
use sea_orm::{ConnectionTrait, Database};
use shaula_core::error::ReasonCode;
use shaula_core::lifecycle::GenerationState as G;
use shaula_core::registry::{ControlPlaneStore, FleetRuntimeGuard, LifecycleStore};
use shaula_core::worker::{CompletionKind, CompletionReceipt, FenceOutcome};
use shaula_daemon::service::ControlPlane;
use shaula_store::http_state::{SqliteStateBackend, WorkerAdmissions};
use shaula_store::registry_impl::SqliteControlPlane;
use shaula_store::Store;
use tower::ServiceExt;
use uuid::Uuid;

use common::attestation_harness::seed_profile;
use common::{authorized, fixed_clock, fixture_engine, oidc, TestPublisher, FLEET_BODY};
use fixture::{
    assert_lines, finalize_req, gen_record, quarantine_with_stale_lock, response_json,
    scrape_metrics, sql, TestResult,
};

#[tokio::test]
async fn finalize_locked_quarantine_and_report_worker_admission_limit() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let artifact_root = tmp.path().join("artifacts");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&artifact_root)?;
    std::fs::create_dir_all(&data_dir)?;

    let db_path = data_dir.join("test.db");
    let store = Store::open(&db_path).await?;
    store.migrate().await?;
    let db = Database::connect(format!("sqlite://{}?mode=rw", db_path.display())).await?;
    let backend = SqliteStateBackend::new(store.clone());
    backend.activate().await?;

    // max_workers=3, recovery_reserve=1 => create_limit=2
    let admissions = Arc::new(WorkerAdmissions::new(3, 1)?);
    let control_plane = Arc::new(
        SqliteControlPlane::new(store.clone(), artifact_root.clone())
            .with_worker_admissions(admissions.clone()),
    );
    let clock = fixed_clock();
    let now = shaula_core::ports::Clock::now_unix_ms(clock.as_ref());
    let service = Arc::new(ControlPlane::new(
        control_plane.clone(),
        clock,
        b"test-bindings-key".to_vec(),
        100,
        fixture_engine(&tmp.path().join("engine")),
    ));
    service.set_ready(true);

    let app = shaula_http::router::build_router(shaula_http::router::AppState {
        access_tokens: None,
        auth_installation_link: None,
        fleets: service.clone(),
        profiles: service.clone(),
        pools: service.clone(),
        health: service.clone(),
        oidc: oidc::oidc().await,
        body_limit: 64 * 1024 * 1024,
        request_body_limit: 64 * 1024 * 1024,
        jobs: None,
        logs: None,
        artifact_publisher: Arc::new(TestPublisher {
            root: artifact_root,
        }),
    });

    let digest = seed_profile(&app, &control_plane, "k8s-linux", true).await;
    let put = app
        .clone()
        .oneshot(authorized(
            "PUT",
            "/api/v1/fleets/f1",
            Some(FLEET_BODY.into()),
        ))
        .await?;
    assert_eq!(put.status(), StatusCode::ACCEPTED);

    let cp: &dyn ControlPlaneStore = control_plane.as_ref();
    let head = cp.fleet_get("f1").await?.ok_or("fleet missing")?;
    let guard = FleetRuntimeGuard::from(&head);

    let gen_fenced = Uuid::new_v4();
    let gen_unfenced = Uuid::new_v4();
    for id in [gen_fenced, gen_unfenced] {
        assert!(
            control_plane
                .generation_insert_guarded(gen_record(&id.to_string(), &digest, now), &guard)
                .await?
        );
    }
    quarantine_with_stale_lock(&backend, &admissions, gen_fenced, FenceOutcome::Fenced).await?;
    quarantine_with_stale_lock(&backend, &admissions, gen_unfenced, FenceOutcome::Unknown).await?;

    // Issue #8: 3rd admission hits create_limit=2 => AdmissionLimitReached (never StorageUnavailable).
    let blocked_err = control_plane
        .generation_insert_guarded(
            gen_record(&Uuid::new_v4().to_string(), &digest, now),
            &guard,
        )
        .await
        .err()
        .ok_or("expected admission limit rejection")?;
    assert_eq!(blocked_err.code, ReasonCode::AdmissionLimitReached);
    for token in [
        "occupancy=2",
        "quarantined=2",
        "create_limit=2",
        "max_workers=3",
        "recovery_reserve=1",
    ] {
        assert!(
            blocked_err.summary.contains(token),
            "expected `{token}` in summary: {}",
            blocked_err.summary
        );
    }

    // Fleet diagnostics capture worker.admission_limit_reached on ScaleUp.
    let mut scale_up_codes: Vec<String> = Vec::new();
    for _ in 0..40 {
        let (status, body) = response_json(
            app.clone()
                .oneshot(authorized("GET", "/api/v1/fleets/f1/diagnostics", None))
                .await?,
        )
        .await?;
        assert_eq!(status, StatusCode::OK);
        scale_up_codes = body["questions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|q| q["question"] == "scale_up")
            .flat_map(|q| q["reasons"].as_array().into_iter().flatten())
            .filter_map(|r| r["code"].as_str().map(str::to_owned))
            .collect();
        if scale_up_codes
            .iter()
            .any(|c| c == "worker.admission_limit_reached")
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(scale_up_codes
        .iter()
        .any(|c| c == "worker.admission_limit_reached"));

    // Prometheus /metrics reports quarantine occupancy and zero headroom.
    let metrics_saturated = scrape_metrics(&store, service.clone(), admissions.limits()).await?;
    assert_lines(
        &metrics_saturated,
        &[
            "shaula_lifecycle_workers{phase=\"quarantined\"} 2",
            "shaula_lifecycle_workers_occupied 2",
            "shaula_lifecycle_workers_quarantined 2",
            "shaula_lifecycle_worker_max 3",
            "shaula_lifecycle_worker_recovery_reserve 1",
            "shaula_lifecycle_worker_create_limit 2",
            "shaula_lifecycle_worker_create_headroom 0",
        ],
    );

    // Issue #7: unfenced quarantined generation MUST stay 409 Conflict with lock intact.
    let (unfenced_status, unfenced_problem) = response_json(
        app.clone()
            .oneshot(finalize_req(
                &gen_unfenced.to_string(),
                "out-of-band check",
            )?)
            .await?,
    )
    .await?;
    assert_eq!(unfenced_status, StatusCode::CONFLICT);
    assert_eq!(
        unfenced_problem["detail"],
        "lifecycle worker must be verifiably fenced before operator finalization"
    );
    let unfenced_row = db
        .query_one(sql(
            "SELECT lock_id, sealed FROM generation_http_state WHERE generation_id = ?",
            vec![gen_unfenced.to_string().into()],
        ))
        .await?
        .ok_or("unfenced state row missing")?;
    assert!(unfenced_row
        .try_get::<Option<String>>("", "lock_id")?
        .is_some());
    assert!(!unfenced_row.try_get::<bool>("", "sealed")?);

    // Issue #7: verifiably fenced quarantined generation with stale lock MUST finalize (202).
    let (fenced_status, fenced_body) = response_json(
        app.clone()
            .oneshot(finalize_req(
                &gen_fenced.to_string(),
                "verified absent out-of-band",
            )?)
            .await?,
    )
    .await?;
    assert_eq!(fenced_status, StatusCode::ACCEPTED, "{fenced_body}");
    let state_row = db
        .query_one(sql(
            "SELECT sealed, revoked, lock_id, lock_info FROM generation_http_state WHERE generation_id = ?",
            vec![gen_fenced.to_string().into()],
        ))
        .await?
        .ok_or("fenced state row missing")?;
    assert!(state_row.try_get::<bool>("", "sealed")?);
    assert!(state_row.try_get::<bool>("", "revoked")?);
    assert!(state_row
        .try_get::<Option<String>>("", "lock_id")?
        .is_none());
    assert!(state_row
        .try_get::<Option<Vec<u8>>>("", "lock_info")?
        .is_none());
    let worker_row = db
        .query_one(sql(
            "SELECT phase, completion_receipt FROM lifecycle_workers WHERE generation_id = ?",
            vec![gen_fenced.to_string().into()],
        ))
        .await?
        .ok_or("fenced worker row missing")?;
    assert_eq!(worker_row.try_get::<String>("", "phase")?, "completed");
    let receipt: CompletionReceipt =
        serde_json::from_str(&worker_row.try_get::<String>("", "completion_receipt")?)?;
    assert_eq!(receipt.kind, CompletionKind::OperatorAttested);
    assert_eq!(
        control_plane
            .generation_get(&gen_fenced.to_string())
            .await?
            .ok_or("generation missing")?
            .state,
        G::Destroyed
    );

    // Finalizing released a slot: headroom=1 and new admission succeeds.
    let metrics_recovered = scrape_metrics(&store, service.clone(), admissions.limits()).await?;
    assert_lines(
        &metrics_recovered,
        &[
            "shaula_lifecycle_workers_occupied 1",
            "shaula_lifecycle_workers_quarantined 1",
            "shaula_lifecycle_worker_create_headroom 1",
        ],
    );
    let gen_unblocked = Uuid::new_v4();
    assert!(
        control_plane
            .generation_insert_guarded(gen_record(&gen_unblocked.to_string(), &digest, now), &guard)
            .await?
    );

    // Issue #7: genuine storage failure during operator_seal_on returns 5xx (never 409) and rolls back.
    quarantine_with_stale_lock(&backend, &admissions, gen_unblocked, FenceOutcome::Fenced).await?;
    db.execute_unprepared(
        "CREATE TRIGGER fail_operator_seal BEFORE UPDATE OF sealed ON generation_http_state
             BEGIN SELECT RAISE(ABORT, 'injected seal failure'); END",
    )
    .await?;
    let (storage_status, storage_problem) = response_json(
        app.clone()
            .oneshot(finalize_req(
                &gen_unblocked.to_string(),
                "expect storage failure",
            )?)
            .await?,
    )
    .await?;
    assert!(
        storage_status.is_server_error(),
        "{storage_status}: {storage_problem}"
    );
    assert_ne!(
        storage_problem["detail"],
        "lifecycle worker must be verifiably fenced before operator finalization"
    );
    assert_eq!(
        control_plane
            .generation_get(&gen_unblocked.to_string())
            .await?
            .ok_or("generation missing")?
            .state,
        G::Quarantined
    );

    let dir =
        std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("quarantine-finalize-admission");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("metrics-saturated.prom"), &metrics_saturated)?;
    std::fs::write(dir.join("metrics-recovered.prom"), &metrics_recovered)?;
    std::fs::write(
        dir.join("report.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "issue_7": {
                "unfenced_status": unfenced_status.as_u16(),
                "fenced_locked_status": fenced_status.as_u16(),
                "storage_error_status": storage_status.as_u16(),
                "storage_error_problem": storage_problem,
            },
            "issue_8": {
                "admission_error_code": blocked_err.code.as_str(),
                "admission_error_summary": blocked_err.summary,
                "diagnostics_scale_up_codes": scale_up_codes,
                "unblocked_after_finalize": true,
            }
        }))?,
    )?;
    Ok(())
}
