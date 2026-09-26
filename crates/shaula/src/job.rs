//! Internal role: no configuration, database, OIDC or management credentials.
use sha2::{Digest, Sha256};
use shaula_core::{
    secret::SecretString,
    state_backend::{StateCapability, StateError, StateResult},
    worker::{wire::LaunchEnvelope, MAX_CONTROL_BYTES},
};
use std::{io::Read, sync::Arc};

pub(crate) fn dispatch() -> i32 {
    match execute() {
        Ok(()) => 0,
        Err(_) => 3,
    }
}

fn execute() -> StateResult<()> {
    if std::env::args_os().count() != 2 {
        return Err(StateError::Invalid);
    }
    let mut stdin = std::io::stdin().lock();
    let mut length = [0; 4];
    stdin
        .read_exact(&mut length)
        .map_err(|_| StateError::Invalid)?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > MAX_CONTROL_BYTES {
        return Err(StateError::TooLarge);
    }
    let mut bytes = zeroize::Zeroizing::new(vec![0; length]);
    stdin
        .read_exact(&mut bytes)
        .map_err(|_| StateError::Invalid)?;
    let launch: LaunchEnvelope = serde_json::from_slice(&bytes).map_err(|_| StateError::Invalid)?;
    drop(bytes);
    drop(stdin);
    validate(&launch)?;
    let client = Arc::new(shaula_worker::WorkerClient::new(&launch)?);
    let backend = shaula_template::http_backend::HttpBackendConfig::new(
        launch.address,
        launch.claim.generation_id,
        StateCapability::parse(SecretString::new(&launch.state_capability))?,
    )
    .map_err(|_| StateError::Invalid)?;
    let runtime = Arc::new(
        shaula_template::TemplateRuntime::with_http_backend(launch.engine.clone(), backend)
            .with_operation_logs(client.clone())
            .with_operation_log_reader(client.clone()),
    );
    // One event-loop thread per waiting Generation, independent of CPU count.
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(2)
        .build()
        .map_err(|_| StateError::Unavailable)?;
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    let _watchdog = std::thread::Builder::new()
        .name("worker-parent".into())
        .spawn(move || {
            let mut extra = [0; 1];
            // EOF or any unexpected second payload closes this worker's admission.
            let _ = std::io::stdin().read(&mut extra);
            let _ = stop.send(true);
        })
        .map_err(|_| StateError::Unavailable)?;
    executor.block_on(async {
        let result = shaula_worker::run(launch, client, runtime.clone(), shutdown).await;
        runtime.drain_operation_logs().await;
        result
    })
}

fn validate(launch: &LaunchEnvelope) -> StateResult<()> {
    launch.validate()?;
    let exe = std::env::current_exe().map_err(|_| StateError::Unavailable)?;
    let bytes = std::fs::read(exe).map_err(|_| StateError::Unavailable)?;
    if format!("sha256:{}", hex::encode(Sha256::digest(bytes))) != launch.executable_digest {
        return Err(StateError::Conflict);
    }
    for (path, root) in [
        (&launch.workspace, &launch.work_root),
        (&launch.artifact, &launch.artifact_root),
    ] {
        if path == &launch.workspace && !path.exists() {
            let parent = path.parent().ok_or(StateError::Invalid)?;
            let parent = std::fs::canonicalize(parent).map_err(|_| StateError::Invalid)?;
            let root = std::fs::canonicalize(root).map_err(|_| StateError::Invalid)?;
            if !parent.starts_with(root) {
                return Err(StateError::Invalid);
            }
            std::fs::create_dir(path).map_err(|_| StateError::Invalid)?;
        }
        let path = std::fs::canonicalize(path).map_err(|_| StateError::Invalid)?;
        let root = std::fs::canonicalize(root).map_err(|_| StateError::Invalid)?;
        if path == root || !path.starts_with(root) {
            return Err(StateError::Invalid);
        }
    }
    Ok(())
}
