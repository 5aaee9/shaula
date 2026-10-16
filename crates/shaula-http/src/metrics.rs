//! The Prometheus scrape surface. It is served on its own loopback-only
//! listener (`observability.metrics_listen`), never on the management
//! router, so the deployment proxy that fronts management HTTP cannot
//! expose it and no credential is needed: reaching it already requires
//! local access to the host.
//!
//! A scrape is the process-lifetime counters recorded through
//! shaula-observability plus gauges read from the ledger at scrape time.
//! Labels are fleet keys and fixed codes; Generation and job IDs, URLs,
//! actor names and credentials never become labels.

use std::sync::Arc;

use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use shaula_core::error::CoreResult;
use shaula_core::ports::ledger_metrics::LedgerMetricsPort;
use shaula_core::registry::{Actor, FleetRegistryPort, FleetStatus, HealthPort, KeyPage, Scope};
use shaula_observability::prometheus::{Exposition, Family, Kind};

/// Prometheus text exposition content type (format 0.0.4).
pub const CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

const fn gauge(name: &'static str, help: &'static str) -> Family {
    Family {
        name,
        help,
        kind: Kind::Gauge,
    }
}

static UP: Family = gauge("shaula_up", "1 while the daemon serves scrapes.");
static BUILD_INFO: Family = gauge("shaula_build_info", "Daemon build version.");
static READY: Family = gauge("shaula_ready", "1 while lifecycle readiness holds.");
static STARTED: Family = gauge(
    "shaula_start_time_seconds",
    "Daemon start time, in Unix seconds.",
);
static DEGRADED: Family = Family {
    name: "shaula_otlp_degraded_exports_total",
    help: "OTLP exports that failed since startup.",
    kind: Kind::Counter,
};
static GENERATIONS: Family = gauge(
    "shaula_generations",
    "Generation ledger rows per fleet and state.",
);
static JOBS: Family = gauge(
    "shaula_jobs",
    "Retained job records per fleet, backend, status and reported result.",
);
static DEMAND: Family = gauge(
    "shaula_fleet_assigned_demand",
    "Jobs currently assigned to the fleet.",
);
static TARGET: Family = gauge(
    "shaula_fleet_capacity_target",
    "Runner target: min(max, min + assigned demand).",
);
static EFFECTIVE: Family = gauge(
    "shaula_fleet_capacity_effective",
    "Generations counting toward effective capacity.",
);
static OCCUPANCY: Family = gauge(
    "shaula_fleet_capacity_occupancy",
    "Generations occupying capacity (every state but Destroyed).",
);
static PHASE: Family = gauge("shaula_fleet_phase", "1 for the fleet's current phase.");
static CONVERGED: Family = gauge(
    "shaula_fleet_converged",
    "1 while the fleet's Converged condition holds.",
);

/// The read ports one scrape consults.
pub struct Scrape {
    pub ledger: Arc<dyn LedgerMetricsPort>,
    pub fleets: Arc<dyn FleetRegistryPort>,
    pub health: Arc<dyn HealthPort>,
    /// Daemon start, in Unix seconds.
    pub started_at: f64,
}

impl Scrape {
    /// One complete scrape body; a failed ledger read fails the scrape as a
    /// whole rather than serving a partial snapshot.
    pub async fn render(&self) -> CoreResult<String> {
        let mut out = Exposition::new();
        out.sample(&UP, &[], 1.0);
        out.sample(&BUILD_INFO, &[("version", env!("CARGO_PKG_VERSION"))], 1.0);
        out.sample(&READY, &[], flag(self.health.ready().await));
        out.sample(&STARTED, &[], self.started_at);
        let degraded = shaula_observability::degraded_exports() as f64;
        out.sample(&DEGRADED, &[], degraded);
        for row in self.ledger.generation_counts().await? {
            let labels = [("fleet", row.fleet_key.as_str()), ("state", &row.state)];
            out.sample(&GENERATIONS, &labels, row.count as f64);
        }
        for job in self.ledger.job_counts().await? {
            let labels = [
                ("fleet", job.fleet_key.as_str()),
                ("backend", job.backend),
                ("status", job.status.as_str()),
                ("result", job_result(job.result.as_deref())),
            ];
            out.sample(&JOBS, &labels, job.count as f64);
        }
        for status in self.fleet_statuses().await? {
            let fleet = [("fleet", status.fleet_key.as_str())];
            let capacity = status.capacity;
            out.sample(&DEMAND, &fleet, capacity.assigned_demand as f64);
            out.sample(&TARGET, &fleet, capacity.target as f64);
            out.sample(&EFFECTIVE, &fleet, capacity.effective as f64);
            out.sample(&OCCUPANCY, &fleet, capacity.occupancy as f64);
            let phase = [fleet[0], ("phase", status.phase.as_str())];
            out.sample(&PHASE, &phase, 1.0);
            let converged = status
                .conditions
                .iter()
                .any(|c| c.condition_type == "Converged" && c.status);
            out.sample(&CONVERGED, &fleet, flag(converged));
        }
        Ok(out.finish())
    }

    /// Every live Fleet's status; one deleted mid-scrape is skipped.
    async fn fleet_statuses(&self) -> CoreResult<Vec<FleetStatus>> {
        let reader = Actor {
            authentication: Default::default(),
            name: "shaula-metrics".into(),
            scopes: vec![Scope::FleetRead],
        };
        let page = KeyPage {
            after: None,
            limit: None,
        };
        let mut statuses = Vec::new();
        for (key, _, _) in self.fleets.fleet_list(&reader, page).await? {
            if let Ok(status) = self.fleets.fleet_status_get(&reader, &key).await? {
                statuses.push(status);
            }
        }
        Ok(statuses)
    }
}

fn flag(value: bool) -> f64 {
    if value {
        1.0
    } else {
        0.0
    }
}

/// Reported results are provider strings; only known conclusions pass.
fn job_result(result: Option<&str>) -> &'static str {
    const KNOWN: [&str; 14] = [
        "success",
        "succeeded",
        "failure",
        "failed",
        "canceled",
        "cancelled",
        "skipped",
        "timed_out",
        "neutral",
        "action_required",
        "stale",
        "manual",
        "abandoned",
        "blocked",
    ];
    let Some(value) = result else { return "none" };
    let lower = value.to_ascii_lowercase();
    KNOWN
        .into_iter()
        .find(|known| *known == lower)
        .unwrap_or("other")
}

/// `GET /metrics` only; every other path is 404.
pub fn router(scrape: Arc<Scrape>) -> Router {
    Router::new().route(
        "/metrics",
        get(move || {
            let scrape = scrape.clone();
            async move {
                match scrape.render().await {
                    Ok(body) => ([(header::CONTENT_TYPE, CONTENT_TYPE)], body).into_response(),
                    Err(error) => {
                        tracing::warn!(summary = %error.summary, "metrics scrape failed");
                        unavailable()
                    }
                }
            }
        }),
    )
}

fn unavailable() -> Response {
    (StatusCode::SERVICE_UNAVAILABLE, "metrics unavailable\n").into_response()
}
