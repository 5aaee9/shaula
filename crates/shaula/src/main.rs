//! Shaula binary: the only composition root. It wires concrete adapters
//! into core ports and contains no fleet reconciliation, SQL, HTTP handler,
//! GitHub protocol or Terraform plan logic.

mod artifact_library;
mod auth_installation_link;
mod auth_worker;
#[cfg(test)]
#[path = "auth_worker_continuity_tests.rs"]
pub(crate) mod auth_worker_continuity_tests;
#[cfg(test)]
#[path = "auth_worker_mock.rs"]
pub(crate) mod auth_worker_mock;
#[cfg(test)]
mod auth_worker_mock_routes;
mod auth_worker_probe;
#[cfg(test)]
#[path = "auth_worker_scheduling_tests.rs"]
pub(crate) mod auth_worker_scheduling_tests;
#[cfg(test)]
mod auth_worker_v2;
mod client_cli;
mod diagnostics;
mod fleet_tasks;
#[cfg(test)]
#[path = "../../shaula-http/tests/support/mod.rs"]
pub(crate) mod http_oidc;
mod job;
mod lifecycle;
mod maintenance;
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
    #[command(flatten)]
    client: client_cli::args::ClientArgs,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Offline, ownership-locked state-format migration.
    Maintenance {
        #[command(subcommand)]
        command: maintenance::Command,
    },
    #[command(flatten)]
    Remote(client_cli::args::RemoteCommand),
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

fn main() {
    if std::env::args_os().nth(1).is_some_and(|arg| arg == "job") {
        std::process::exit(job::dispatch());
    }
    run_cli();
}

#[tokio::main]
async fn run_cli() {
    if let Some(code) = shaula_template::ssh_helper::dispatch() {
        std::process::exit(code);
    }
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
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) if !error.use_stderr() => error.exit(),
        Err(_) => {
            // Clap diagnostics can echo unexpected argv. Keep machine errors
            // stable and never echo a mistakenly supplied credential value.
            let args = client_cli::args::ClientArgs {
                output: Some(client_cli::args::Output::Json),
                ..Default::default()
            };
            let outcome = client_cli::Outcome::error(shaula_client::Error::Invalid(
                "invalid command arguments; use --help for usage",
            ));
            client_cli::output(&args, "parse", &outcome);
            std::process::exit(2);
        }
    };
    match cli.command {
        Command::Maintenance { command } => {
            if let Err(error) = maintenance::run(command).await {
                eprintln!("shaula maintenance: {error}");
                std::process::exit(1);
            }
        }
        Command::Remote(command) => std::process::exit(client_cli::run(cli.client, command).await),
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
