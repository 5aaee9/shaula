//! Shaula binary: the only composition root. It wires concrete adapters
//! into core ports and contains no fleet reconciliation, SQL, HTTP handler,
//! GitHub protocol or Terraform plan logic.

mod artifact_library;
mod auth_worker;
#[cfg(test)]
#[path = "auth_worker_continuity_tests.rs"]
pub(crate) mod auth_worker_continuity_tests;
#[cfg(test)]
#[path = "auth_worker_mock.rs"]
pub(crate) mod auth_worker_mock;
#[cfg(test)]
mod auth_worker_mock_routes;
mod auth_worker_predecessor;
mod auth_worker_probe;
#[cfg(test)]
#[path = "auth_worker_scheduling_tests.rs"]
pub(crate) mod auth_worker_scheduling_tests;
mod auth_worker_selectors;
mod auth_worker_v2;
mod fleet_tasks;
mod oidc_args;
mod wiring;
mod wiring_credential;

use clap::{Parser, Subcommand};
use shaula_core::ports::Clock;
use shaula_daemon::bootstrap::ValidatedBootstrap;
use shaula_store::registry_impl::SqliteControlPlane;
use shaula_store::Store;
use shaula_template::TemplateRuntime;

/// System clock adapter for the composition root.
struct SystemClock;

impl Clock for SystemClock {
    fn now_unix_ms(&self) -> i64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or_default()
    }
}

#[derive(Parser)]
#[command(
    name = "shaula",
    version,
    about = "GitHub Actions Scale Set capacity controller"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the HTTP control plane and all active fleets.
    Serve {
        /// Path to the daemon bootstrap configuration file.
        #[arg(long)]
        config: String,
        #[command(flatten)]
        oidc: oidc_args::OidcArgs,
    },
    /// Print version information.
    Version,
}

#[tokio::main]
async fn main() {
    // The engine fence supervisor (R7-06/R9-09) re-executes THIS binary
    // with a hidden internal marker. It must never reach the CLI parser
    // and must never log: its stdio belongs to the engine invocation.
    {
        let raw: Vec<String> = std::env::args().skip(1).collect();
        if raw.first().map(String::as_str)
            == Some(shaula_template::engine::engine_supervisor::FENCE_ARG)
        {
            std::process::exit(
                shaula_template::engine::engine_supervisor::run_fence_supervisor(&raw[1..]),
            );
        }
    }
    let cli = Cli::parse();
    match cli.command {
        Command::Version => {
            println!("shaula {}", env!("CARGO_PKG_VERSION"));
        }
        Command::Serve { config, oidc } => match serve(&config, oidc).await {
            Ok(()) => println!("shaula stopped cleanly"),
            Err(err) => {
                eprintln!("shaula: {err}");
                std::process::exit(1);
            }
        },
    }
}

async fn serve(config_path: &str, oidc: oidc_args::OidcArgs) -> Result<(), String> {
    // Bootstrap validation happens before the API becomes ready; runtime
    // errors never print clap usage. Validation is a pure file read, so
    // telemetry can initialize once with the CONFIGURED service name and
    // still precede every migration or remote effect (spec 0001 §13).
    let bootstrap = ValidatedBootstrap::load(std::path::Path::new(config_path))?;
    let _telemetry = shaula_observability::init(&bootstrap.service_name);
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
    let runtime: std::sync::Arc<dyn shaula_core::ports::TemplateRuntimePort> =
        std::sync::Arc::new(TemplateRuntime::new(bootstrap.terraform_executable.clone()));
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
    );
    let (shutdown_tx, wiring_shutdown) = tokio::sync::watch::channel(false);
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
        fleets: service.clone(),
        profiles: service.clone(),
        health: service.clone(),
        oidc,
        body_limit: bootstrap.artifact_body_limit,
        request_body_limit: bootstrap.request_body_limit,
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
    let mut scan_shutdown = shutdown_tx.subscribe();
    let mut scan_handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(shaula_daemon::daemon::SCAN_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let scan_store = scan_store.clone();
                    let tick_clock = clock.clone();
                    let mut scan = tokio::spawn(async move {
                        scan_store.periodic_scan(tick_clock.now_unix_ms()).await
                    });
                    // The in-flight tick is itself cancellable: shutdown
                    // aborts it so no inner task can outlive the loop and
                    // touch the database after the ownership lock drops
                    // (SeaORM queries are cancel-safe).
                    tokio::select! {
                        result = &mut scan => {
                            match result {
                                Ok(Ok(_report)) => {}
                                Ok(Err(e)) => {
                                    tracing::warn!(summary = %e.summary, "periodic scan failed");
                                }
                                Err(join_err) => {
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
    drop(_lock);

    // Fold the server outcome; the scan/wiring results are supervision
    // detail (panics were already logged) and never mask the exit status.
    let _ = scan_final;
    let _ = wiring_final;
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
