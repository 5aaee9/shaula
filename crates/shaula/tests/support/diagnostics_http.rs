use crate::common;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use sea_orm::{ConnectionTrait, Database, DatabaseBackend::Sqlite, Statement};
use shaula_core::{
    diagnostics::*,
    registry::{ControlPlaneStore, FleetRuntimeGuard},
};
use tower::ServiceExt;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[tokio::test]
async fn real_router_store_reports_permissions_queries_expiry_and_storage_failure() -> TestResult {
    let (app, store, engine) = common::build_app_with_scan().await;
    let root = engine
        .parent()
        .and_then(std::path::Path::parent)
        .ok_or("root")?;
    let db = Database::connect(format!(
        "sqlite://{}?mode=rw",
        root.join("data/test.db").display()
    ))
    .await?;
    db.execute_unprepared("INSERT INTO fleets(key,incarnation,desired_revision,mutation_fence,created_at,updated_at) VALUES('explain','original',1,1,1,1)").await?;
    let guard = Guard::fleet(
        "explain",
        &FleetRuntimeGuard {
            incarnation: "original".into(),
            desired_revision: 1,
            mutation_fence: 1,
        },
    );
    let observer = Observer::register(
        store.diagnostic_sink(),
        guard,
        Lane::Supervisor,
        QuestionId::ScaleUp,
    )
    .ok_or("observer")?;
    let now = 1_800_000_000_000;
    let mut ticket = observer.begin(now).ok_or("ticket")?;
    ticket.observation.question.capacity = Some(capacity(
        shaula_core::capacity::CapacityPolicy {
            min_runners: 2,
            max_runners: 10,
        },
        Some(5),
        shaula_core::capacity::CapacityCounters {
            effective_capacity: 4,
            resource_occupancy: 10,
        },
        DemandKind::GithubTotalAssignedJobs,
    ));
    ticket.observation.question.reason(
        Code::CapacityOccupancyLimit,
        StageId::Capacity,
        Effect::Blocking,
        EvidenceKind::DerivedCalculation,
    );
    ticket.observation.question.pool = Some(DiagnosticPool {
        mode: RoutingMode::Shared,
        scope: OccupancyScope::PoolRevision,
        pool_revision: Some("7".into()),
        members: vec![DiagnosticPoolMember {
            key: "private-member".into(),
            weight: "2".into(),
            cap: Some("10".into()),
            occupancy: "10".into(),
            excluded_at_cap: true,
        }],
        selected_member: None,
        selection: SelectionOutcome::NoEligibleMember,
        truncated: false,
    });
    ticket.publish();
    let uri = "/api/v1/fleets/explain/diagnostics";
    let mut report = serde_json::Value::Null;
    for _ in 0..100 {
        let response = app
            .clone()
            .oneshot(common::authorized("GET", uri, None))
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "private, no-store");
        assert!(!response.headers().contains_key("etag"));
        assert!(!response.headers().contains_key("shaula-resource-version"));
        report =
            serde_json::from_slice(&axum::body::to_bytes(response.into_body(), 256 * 1024).await?)?;
        if report["questions"][0]["outcome"] == "blocked" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(report["questions"][0]["capacity"]["target"], "7");
    assert_eq!(
        report["questions"][0]["capacity"]["arithmeticCreateAllowance"],
        "0"
    );
    assert_eq!(
        report["questions"][0]["stages"][4]["evaluation"],
        "not_evaluated"
    );
    for (scopes, details) in [("fleet.read", false), ("fleet.read template.read", true)] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header("authorization", common::oidc::bearer(scopes))
                    .body(Body::empty())?,
            )
            .await?;
        let body = axum::body::to_bytes(response.into_body(), 256 * 1024).await?;
        assert_eq!(
            std::str::from_utf8(&body)?.contains("private-member"),
            details
        );
    }
    for scopes in ["fleet.write", "auth.read", "logs.read", ""] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header("authorization", common::oidc::bearer(scopes))
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(response.headers()["cache-control"], "private, no-store");
    }
    let response = app
        .clone()
        .oneshot(common::authorized(
            "GET",
            &format!("{uri}?refresh=true"),
            None,
        ))
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    db.execute_unprepared("UPDATE fleets SET tombstone=1 WHERE key='explain'")
        .await?;
    let gone = app
        .clone()
        .oneshot(common::authorized("GET", uri, None))
        .await?;
    assert_eq!(gone.status(), StatusCode::GONE);
    assert_eq!(gone.headers()["cache-control"], "private, no-store");
    db.execute_unprepared("UPDATE fleets SET tombstone=0 WHERE key='explain'")
        .await?;
    // A failed optional projection remains a successful, incomplete read.
    db.execute_unprepared("DROP TABLE diagnostic_snapshots")
        .await?;
    let response = app
        .clone()
        .oneshot(common::authorized("GET", uri, None))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    // The authoritative store failing is not converted into an empty success.
    db.execute_unprepared("ALTER TABLE fleets RENAME TO unavailable_fleets")
        .await?;
    let response = app.oneshot(common::authorized("GET", uri, None)).await?;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    Ok(())
}

#[tokio::test]
async fn generation_and_job_reports_do_not_leak_logs_or_invent_associations() -> TestResult {
    let (app, _, engine) = common::build_app_with_scan().await;
    let root = engine
        .parent()
        .and_then(std::path::Path::parent)
        .ok_or("root")?;
    let db = Database::connect(format!(
        "sqlite://{}?mode=rw",
        root.join("data/test.db").display()
    ))
    .await?;
    db.execute_unprepared("INSERT INTO fleets(key,incarnation,created_at,updated_at) VALUES('explain','new-incarnation',1,1);
        INSERT INTO runner_generations(id,fleet_key,runner_name,generation_name,fleet_revision,template_profile_key,template_revision,template_artifact_digest,attestation_id,inputs_digest,state,workspace_path,created_at,updated_at)
        VALUES('gen','explain','runner','resource',1,'private-template',1,'private-digest','private-attestation','private-inputs','Quarantined','secret-workspace',1,1)").await?;
    let summary = serde_json::json!({"id":"job","fleet_key":"explain","fleet_incarnation":"old-incarnation",
        "protocol_job_id":"opaque","observed_status":"queued","freshness":"unknown","association_status":"ambiguous",
        "created_at":1,"updated_at":1,"job_display_name":"<script>secret-token</script>"});
    db.execute(Statement::from_sql_and_values(Sqlite,
        "INSERT INTO workflow_jobs(id,scope_key,fleet_key,fleet_incarnation,scale_set_id,protocol_job_id,summary_json,status,created_at,updated_at) VALUES('job','scope','explain','old-incarnation',1,'opaque',?,'queued',1,1)", vec![summary.to_string().into()])).await?;
    for path in [
        "/api/v1/generations/gen/diagnostics",
        "/api/v1/jobs/job/diagnostics",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("authorization", common::oidc::bearer("fleet.read"))
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 256 * 1024).await?;
        let text = std::str::from_utf8(&bytes)?;
        for secret in [
            "private-template",
            "private-digest",
            "private-attestation",
            "private-inputs",
            "secret-workspace",
            "<script>",
            "secret-token",
            "new-incarnation",
        ] {
            assert!(!text.contains(secret), "{secret}");
        }
        let report: DiagnosticReportV1 = serde_json::from_slice(&bytes)?;
        assert!(report.related.is_empty());
        if path.contains("/jobs/") {
            assert_eq!(
                report.subject.fleet_incarnation.as_deref(),
                Some("old-incarnation")
            );
            assert_eq!(report.questions[0].outcome, Outcome::Unknown);
        }
    }
    Ok(())
}
