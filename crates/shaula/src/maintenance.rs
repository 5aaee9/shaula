//! Offline migration uses the same data-directory lock as serve. No remote
//! credentials, acquisition, Terraform invocation or implicit state selection.
mod inspect;
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use shaula_daemon::bootstrap::ValidatedBootstrap;
use shaula_store::{
    http_state::{MigrationClassification, SqliteStateBackend},
    Store,
};
use std::{io::Write, path::PathBuf};

#[derive(Subcommand)]
pub enum Command {
    LifecycleState {
        #[command(subcommand)]
        command: Action,
    },
}
#[derive(Subcommand)]
pub enum Action {
    Inspect {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        plan: PathBuf,
        /// Original service cgroup, after stopping its daemon and descendants.
        #[arg(long)]
        legacy_cgroup: Option<PathBuf>,
    },
    Apply {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        legacy_cgroup: Option<PathBuf>,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    version: u32,
    deployment: String,
    engine: String,
    fence: Option<String>,
    generations: Vec<Entry>,
}
#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: String,
    commitment: String,
    classification: MigrationClassification,
    reason: String,
}

pub async fn run(command: Command) -> Result<(), String> {
    let Command::LifecycleState { command } = command;
    let (config, path, cgroup, apply) = match command {
        Action::Inspect {
            config,
            plan,
            legacy_cgroup,
        } => (config, plan, legacy_cgroup, false),
        Action::Apply {
            config,
            plan,
            legacy_cgroup,
        } => (config, plan, legacy_cgroup, true),
    };
    let bootstrap = ValidatedBootstrap::load(&config)?;
    let _lock = shaula_daemon::daemon::acquire_ownership_lock(&bootstrap.data_dir)?;
    shaula_daemon::bootstrap::verify_storage_containment(&bootstrap)?;
    let store = Store::open(&bootstrap.database_path)
        .await
        .map_err(|_| "database unavailable")?;
    store
        .migrate()
        .await
        .map_err(|_| "schema migration failed")?;
    let backend = SqliteStateBackend::new(store);
    let deployment = backend
        .deployment_identity()
        .await
        .map_err(|_| "deployment identity unavailable")?;
    use sha2::{Digest, Sha256};
    let engine = format!(
        "sha256:{}",
        hex::encode(Sha256::digest(
            std::fs::read(&bootstrap.terraform_executable)
                .map_err(|_| "engine identity unavailable")?
        ))
    );
    let fence = cgroup
        .as_deref()
        .map(shaula_executor::legacy_fence)
        .transpose()
        .map_err(|_| "legacy service containment is not verifiably empty")?;
    let generations = backend
        .legacy_generations()
        .await
        .map_err(|_| "legacy scan failed")?;
    if generations.len() > 4096 {
        return Err("migration plan exceeds 4096 generations".into());
    }
    if !apply {
        let mut entries = Vec::new();
        for generation in &generations {
            entries.push(inspect::inspect(generation, &bootstrap, &engine, fence.is_some())?.entry);
        }
        let plan = Plan {
            version: 1,
            deployment,
            engine,
            fence,
            generations: entries,
        };
        let bytes = serde_json::to_vec_pretty(&plan).map_err(|_| "plan encoding failed")?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        let mut file =
            tempfile::NamedTempFile::new_in(parent).map_err(|_| "plan directory unavailable")?;
        file.write_all(&bytes)
            .and_then(|()| file.as_file().sync_all())
            .map_err(|_| "plan write failed")?;
        file.persist_noclobber(&path)
            .map_err(|_| "plan already exists or cannot be published")?;
        println!(
            "inspected {} generations; plan contains commitments only",
            plan.generations.len()
        );
        return Ok(());
    }
    if std::fs::metadata(&path)
        .map_err(|_| "plan unavailable")?
        .len()
        > 4 * 1024 * 1024
    {
        return Err("plan too large".into());
    }
    let plan: Plan = serde_json::from_slice(&std::fs::read(&path).map_err(|_| "plan unavailable")?)
        .map_err(|_| "invalid plan")?;
    if plan.version != 1
        || plan.deployment != deployment
        || plan.engine != engine
        || plan.fence != fence
    {
        return Err("stale migration plan or fence evidence".into());
    }
    let mut unique = std::collections::HashSet::new();
    if plan.generations.len() > 4096
        || plan
            .generations
            .iter()
            .any(|entry| !unique.insert(&entry.id))
    {
        return Err("duplicate or excessive migration entries".into());
    }
    let mut pending = Vec::new();
    for entry in &plan.generations {
        if backend
            .import_receipt(&entry.id, &entry.commitment)
            .await
            .map_err(|_| "migration receipt conflict")?
        {
            continue;
        }
        let generation = generations
            .iter()
            .find(|g| g.id == entry.id)
            .ok_or("generation set changed since inspect")?;
        let inspected = inspect::inspect(generation, &bootstrap, &engine, fence.is_some())?;
        if inspected.entry != *entry {
            return Err("source state, inputs, artifact or database changed since inspect".into());
        }
        pending.push((generation, entry));
    }
    if generations.iter().any(|generation| {
        !plan
            .generations
            .iter()
            .any(|entry| entry.id == generation.id)
    }) {
        return Err("unclassified generation outside plan".into());
    }
    for (generation, entry) in pending {
        if cgroup
            .as_deref()
            .map(shaula_executor::legacy_fence)
            .transpose()
            .map_err(|_| "legacy fence lost")?
            != fence
        {
            return Err("legacy fence changed".into());
        }
        // Re-read immediately before EACH short commit, including resumptions;
        // a prior batch-wide check does not commit later source-file contents.
        let inspected = inspect::inspect(generation, &bootstrap, &engine, fence.is_some())?;
        if inspected.entry != *entry {
            return Err("source changed during apply; earlier receipts remain replayable".into());
        }
        let target = bootstrap.work_root.join("http").join(&generation.id);
        if inspected.entry.classification == MigrationClassification::Imported {
            std::fs::create_dir_all(target.parent().ok_or("workspace parent missing")?)
                .map_err(|_| "recovery root unavailable")?;
        }
        backend
            .import_legacy(
                generation,
                &inspected.entry.commitment,
                inspected.state,
                inspected.entry.classification,
                inspected.input,
                target.to_str(),
            )
            .await
            .map_err(|_| "generation migration failed; committed receipts can be replayed")?;
    }
    backend
        .activate()
        .await
        .map_err(|_| "activation blocked by unclassified generations")?;
    println!("lifecycle HTTP state activated; legacy evidence retained");
    Ok(())
}
