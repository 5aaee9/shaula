//! The `shaula serve` composition path: telemetry startup, adapter
//! wiring, the supervised scan loop and bounded shutdown ordering.

use std::time::Duration;

use shaula_core::ports::Clock;
use shaula_daemon::bootstrap::ValidatedBootstrap;
use shaula_store::registry_impl::SqliteControlPlane;
use shaula_store::Store;
use shaula_template::TemplateRuntime;
use tracing::Instrument;

use crate::{
    artifact_library, auth_installation_link, diagnostics, oidc_args, wiring, SystemClock,
};

pub(crate) async fn serve(config_path: &str, oidc: oidc_args::OidcArgs) -> Result<(), String> {
    // Bootstrap validation happens before the API becomes ready; runtime
    // errors never print clap usage. Validation is a pure file read, so
    // telemetry can initialize once with the CONFIGURED service name and
    // still precede every migration or remote effect (spec 0001 §13).
    let bootstrap = ValidatedBootstrap::load(std::path::Path::new(config_path))?;
    let _telemetry =
        shaula_observability::init(&bootstrap.service_name, bootstrap.otlp_endpoint.as_deref());
    // Telemetry is live: the startup span covers recovery and listener
    // launch (spec 0001 §13.1).
    let startup_span = tracing::info_span!("shaula.daemon.startup");
    let _startup = startup_span.enter();
    oidc.validate_setup_origin(bootstrap.setup_info.as_ref())?;
    let oidc = oidc.initialize(bootstrap.authorization.clone()).await?;

    // Filesystem roots must exist before the store opens its database.
    for dir in [
        &bootstrap.data_dir,
        &bootstrap.work_root,
        &bootstrap.artifact_root,
    ] {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("root {} unusable: {e}", dir.display()))?;
    }
    // Ownership lock FIRST (single-writer invariant, spec 0001 §11.1),
    // then open + migrate the store under that exclusive ownership.
    let _lock = shaula_daemon::daemon::acquire_ownership_lock(&bootstrap.data_dir)?;
    // R9-11: canonical containment — symlinks/junctions or absolute
    // redirections that would point the ledger (or any root) outside this
    // locked data dir fail closed here, before the store opens.
    shaula_daemon::bootstrap::verify_storage_containment(&bootstrap)?;
    let store = Store::open(&bootstrap.database_path)
        .await
        .map_err(|e| e.to_string())?;
    store.migrate().await.map_err(|e| e.to_string())?;
    let diagnostics = diagnostics::Diagnostics::new(store.clone(), &bootstrap).await;
    let jobs = std::sync::Arc::new(store.clone());
    let logs = diagnostics.logs.clone();

    let clock: std::sync::Arc<dyn Clock> = std::sync::Arc::new(SystemClock);
    let artifact_library = std::sync::Arc::new(artifact_library::DbArtifactPublisher::new(
        store.clone(),
        bootstrap.artifact_root.clone(),
    ));
    artifact_library
        .initialize(&bootstrap.template_source_dirs, clock.now_unix_ms())
        .await
        .map_err(|error| error.summary)?;
    let control_plane_store = std::sync::Arc::new(
        SqliteControlPlane::new(store, bootstrap.artifact_root.clone())
            .with_artifact_cache(artifact_library.clone()),
    );

    // Control-plane service implementing the registry ports. The engine
    // binary is the attestation subject's binary-digest authority.
    let service = std::sync::Arc::new(shaula_daemon::service::ControlPlane::new(
        control_plane_store.clone(),
        clock.clone(),
        bootstrap.bindings_server_key.clone().into_bytes(),
        bootstrap.max_active_fleets,
        bootstrap.terraform_executable.clone(),
    ));

    // Start the scheduler before serving HTTP. Remote Fleet work runs in
    // independent tasks; readiness cannot wait for a GitHub/IaC round trip.
    let mut runtime = TemplateRuntime::new(bootstrap.terraform_executable.clone());
    if let Some(logs) = &logs {
        runtime = runtime
            .with_operation_logs(logs.clone())
            .with_operation_log_reader(logs.clone());
    }
    let runtime = std::sync::Arc::new(runtime);
    let runtime_logs = runtime.clone();
    let lifecycle: std::sync::Arc<dyn shaula_core::registry::LifecycleStore> =
        control_plane_store.clone();
    let wiring = wiring::SupervisorWiring::new(
        control_plane_store.clone(),
        lifecycle,
        runtime,
        clock.clone(),
        service.effect_gates(),
        bootstrap.work_root.clone(),
        bootstrap.artifact_root.clone(),
        bootstrap.operation_timeout,
        shaula_daemon::supervisor::LifecycleLimits {
            create: std::sync::Arc::new(tokio::sync::Semaphore::new(bootstrap.create_concurrency)),
            destroy: std::sync::Arc::new(tokio::sync::Semaphore::new(
                bootstrap.destroy_concurrency,
            )),
        },
    )
    .with_setup_info_issuer(diagnostics.issuer.clone());
    let (shutdown_tx, wiring_shutdown) = tokio::sync::watch::channel(false);
    let diagnostics_task = tokio::spawn(diagnostics.run(shutdown_tx.subscribe()));
    let mut wiring_task = tokio::spawn(wiring.run(wiring_shutdown));
    service.set_ready(true);

    // HTTP server (loopback-only; validated at config parse and bind).
    let (host, port) = bootstrap
        .listen
        .rsplit_once(':')
        .map(|(h, p)| p.parse::<u16>().map(|port| (h.to_string(), port)))
        .transpose()
        .map_err(|e| format!("listen port invalid: {e}"))?
        .unwrap_or_else(|| ("127.0.0.1".to_string(), 8080));
    let http_config =
        shaula_http::server::ServerConfig::new(host, port, bootstrap.request_body_limit)?;
    let state = shaula_http::router::AppState {
        auth_installation_link: Some(std::sync::Arc::new(
            auth_installation_link::StoredAuthInstallationLink::production(
                control_plane_store.clone(),
                clock.clone(),
            )
            .map_err(|_| "GitHub App installation link client could not be initialized")?,
        )),
        fleets: service.clone(),
        profiles: service.clone(),
        health: service.clone(),
        oidc,
        body_limit: bootstrap.artifact_body_limit,
        request_body_limit: bootstrap.request_body_limit,
        jobs: Some(jobs),
        logs: logs.map(|archive| {
            archive as std::sync::Arc<dyn shaula_core::operation_log::OperationLogReadPort>
        }),
        artifact_publisher: artifact_library,
    };
    let app = shaula_http::router::build_router(state);

    // Level-triggered scan loop: consumes outbox markers and runs static
    // validation over Template Candidates. The task is SUPERVISED: each
    // tick runs in its own task so a panic is caught by the JoinError arm,
    // marks readiness false (degraded, not silent) and leaves the loop
    // alive to retry; shutdown drains it before the server stops.
    let scan_store = control_plane_store.clone();
    let scan_ready = service.clone();
    let scan_cascade = service.clone();
    let mut scan_shutdown = shutdown_tx.subscribe();
    let mut scan_handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(shaula_daemon::daemon::SCAN_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let scan_store = scan_store.clone();
                    let scan_cascade = scan_cascade.clone();
                    let tick_clock = clock.clone();
                    // One span per tick: the scan work only (a successful
                    // empty poll is not a long-lived span, spec 0001 §13.1).
                    let tick_span = tracing::info_span!("shaula.fleet.reconcile_tick");
                    let mut scan = tokio::spawn(
                        async move {
                        let now = tick_clock.now_unix_ms();
                        let scan_result = scan_store.periodic_scan(now).await;
                        // Follower fleets upgrade on the same level-triggered
                        // cadence (spec 0023): independent of scan outcome.
                        let cascade_result = scan_cascade
                            .cascade_template_follow_upgrades(now)
                            .await;
                        (scan_result, cascade_result)
                        }
                        .instrument(tick_span),
                    );
                    // The in-flight tick is itself cancellable: shutdown
                    // aborts it so no inner task can outlive the loop and
                    // touch the database after the ownership lock drops
                    // (SeaORM queries are cancel-safe).
                    tokio::select! {
                        result = &mut scan => {
                            match result {
                                Ok((scan_result, cascade_result)) => {
                                    let outcome = if scan_result.is_err() || cascade_result.is_err() {
                                        shaula_observability::MetricResult::Failed
                                    } else {
                                        shaula_observability::MetricResult::Ok
                                    };
                                    shaula_observability::TelemetryHandle::new().record(
                                        shaula_observability::MetricOperation::Reconcile,
                                        outcome,
                                        1,
                                    );
                                    if let Err(e) = scan_result {
                                        tracing::warn!(summary = %e.summary, "periodic scan failed");
                                    }
                                    if let Err(e) = cascade_result {
                                        tracing::warn!(summary = %e.summary, "template follow cascade failed");
                                    }
                                }
                                Err(join_err) => {
                                    shaula_observability::TelemetryHandle::new().record(
                                        shaula_observability::MetricOperation::Reconcile,
                                        shaula_observability::MetricResult::Failed,
                                        1,
                                    );
                                    tracing::error!(summary = %join_err, "periodic scan panicked");
                                    scan_ready.set_ready(false);
                                }
                            }
                        }
                        _ = scan_shutdown.changed() => {
                            // Fence the in-flight tick AND await its
                            // termination before leaving the loop: abort
                            // alone does not prove the task stopped
                            // touching the database (F03).
                            scan.abort();
                            let _ = (&mut scan).await;
                            break;
                        }
                    }
                }
                _ = scan_shutdown.changed() => break,
            }
        }
    });

    // The composition root is the SINGLE cancel owner: the server task
    // never listens for Ctrl-C itself, it only obeys the watch channel.
    // The server runs supervised so a bind failure (port occupied) or serve
    // error propagates as a daemon-fatal exit instead of a clean stop.
    // ("shaula listening" is printed by the server AFTER a successful
    // bind — never before the outcome is known.)
    let server_shutdown = shutdown_tx.subscribe();
    let mut server = tokio::spawn(shaula_http::server::serve(
        app,
        http_config.listen_addr(),
        server_shutdown,
    ));
    // Listeners and workers are launched: startup is complete.
    drop(_startup);

    let mut scheduler_stopped = false;
    let server_result = tokio::select! {
        _ = &mut wiring_task => {
            scheduler_stopped = true;
            service.set_ready(false);
            None
        }
        result = &mut server => Some(result),
        _ = tokio::signal::ctrl_c() => {
            println!("shaula shutting down");
            None
        }
    };

    // UNIFIED quiescence for every exit path (clean stop, server-fatal,
    // Ctrl-C): trigger shutdown once, then consume EACH JoinHandle exactly
    // once — a JoinHandle polled after completion panics, so a handle the
    // select already drained is never awaited again (F03). A handle still
    // running at the deadline is aborted AND awaited so its termination
    // is proven before the ownership lock drops.
    let shutdown_span = tracing::info_span!("shaula.daemon.shutdown");
    let _shutdown = shutdown_span.enter();
    let _ = shutdown_tx.send(true);
    let drain = shaula_daemon::daemon::SCAN_INTERVAL;
    let server_join = match server_result {
        // The select already consumed the server handle: its outcome IS
        // the join result.
        some @ Some(_) => some,
        None => match tokio::time::timeout(drain, &mut server).await {
            Ok(joined) => Some(joined),
            Err(_elapsed) => {
                server.abort();
                Some(server.await)
            }
        },
    };
    let scan_final = match tokio::time::timeout(drain, &mut scan_handle).await {
        Ok(joined) => Some(joined),
        Err(_elapsed) => {
            scan_handle.abort();
            Some(scan_handle.await)
        }
    };
    // Wiring cancels its scan and awaits every owned Fleet task on shutdown.
    // Do not abort that owner while it is proving child quiescence.
    let wiring_final = if scheduler_stopped {
        None
    } else {
        Some(wiring_task.await)
    };
    runtime_logs.drain_operation_logs().await;
    let _ = diagnostics_task.await;
    drop(_lock);

    // Fold the server outcome; the scan/wiring results are supervision
    // detail (panics were already logged) and never mask the exit status.
    let _ = scan_final;
    let _ = wiring_final;
    // Shutdown work is done: close the span and give the trace pipeline a
    // bounded window to export what is buffered (spec 0001 §12, §13).
    drop(_shutdown);
    _telemetry.flush(Duration::from_secs(2)).await;
    if scheduler_stopped {
        return Err("supervisor scheduler stopped unexpectedly".into());
    }
    match server_join {
        None => Ok(()),
        Some(Ok(Ok(()))) => Ok(()),
        Some(Ok(Err(e))) => Err(e),
        Some(Err(join)) => Err(format!("server task failed: {join}")),
    }
}
