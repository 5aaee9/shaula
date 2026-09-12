//! Shaula binary: the only composition root. It wires concrete adapters
//! into core ports and contains no fleet reconciliation, SQL, HTTP handler,
//! GitHub protocol or Terraform plan logic.

mod artifact_library;
mod auth_installation_link;
mod auth_worker;
#[cfg(test)]
#[path = "auth_worker_continuity_tests.rs"]
pub(crate) mod auth_worker_continuity_tests;
mod auth_worker_forgejo;
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
mod diagnostics;
mod fleet_tasks;
#[cfg(test)]
#[path = "../../shaula-http/tests/support/mod.rs"]
pub(crate) mod http_oidc;
mod oidc_args;
mod serve;
mod wiring;
mod wiring_credential;

use clap::{Parser, Subcommand};
use shaula_core::ports::Clock;

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
        Command::Serve { config, oidc } => match serve::serve(&config, oidc).await {
            Ok(()) => println!("shaula stopped cleanly"),
            Err(err) => {
                eprintln!("shaula: {err}");
                std::process::exit(1);
            }
        },
    }
}
