//! Explicit worker HTTP backend configuration. No address or credential is
//! accepted from Template files, command-line backend flags or inherited env.

use std::{collections::HashSet, io::Write, net::SocketAddr, path::Path};

use shaula_core::{
    error::{CoreError, CoreResult, ReasonCode},
    state_backend::StateCapability,
};
use uuid::Uuid;

pub(crate) const BACKEND_FILE: &str = "shaula.backend.tf";
const BACKEND_HCL: &[u8] = b"terraform {\n  backend \"http\" {}\n}\n";

#[derive(Debug, Clone)]
pub struct HttpBackendConfig {
    address: SocketAddr,
    generation_id: Uuid,
    capability: StateCapability,
}

fn invalid() -> CoreError {
    CoreError::new(
        ReasonCode::TemplateInvalid,
        "HTTP backend configuration or workspace invalid",
    )
}

impl HttpBackendConfig {
    pub fn new(
        address: SocketAddr,
        generation_id: Uuid,
        capability: StateCapability,
    ) -> CoreResult<Self> {
        if !address.ip().is_loopback() || address.port() == 0 {
            return Err(invalid());
        }
        Ok(Self {
            address,
            generation_id,
            capability,
        })
    }

    pub fn address(&self) -> String {
        format!(
            "http://{}/internal/v1/generations/{}/state",
            self.address, self.generation_id
        )
    }

    pub(crate) fn check_generation(&self, generation: &str) -> CoreResult<()> {
        if generation != self.generation_id.to_string() {
            return Err(invalid());
        }
        Ok(())
    }

    /// The resulting vector is a secret handoff to env_clear() Terraform
    /// children only. The control capability is never an input to this method.
    pub fn environment(
        &self,
        provider_env: &[(String, String)],
    ) -> CoreResult<Vec<(String, String)>> {
        let mut seen = HashSet::new();
        for (name, _) in provider_env {
            let name = name.to_ascii_uppercase();
            if name.is_empty()
                || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
                || !seen.insert(name.clone())
                || ["TF_", "SHAULA_", "GITHUB_", "ACTIONS_", "OIDC_"]
                    .iter()
                    .any(|p| name.starts_with(p))
                || matches!(
                    name.as_str(),
                    "HTTP_PROXY" | "HTTPS_PROXY" | "ALL_PROXY" | "NO_PROXY"
                )
            {
                return Err(invalid());
            }
        }
        let address = self.address();
        let mut env = provider_env.to_vec();
        env.extend([
            ("TF_HTTP_ADDRESS".into(), address.clone()),
            ("TF_HTTP_LOCK_ADDRESS".into(), address.clone()),
            ("TF_HTTP_UNLOCK_ADDRESS".into(), address),
            ("TF_HTTP_UPDATE_METHOD".into(), "POST".into()),
            ("TF_HTTP_LOCK_METHOD".into(), "LOCK".into()),
            ("TF_HTTP_UNLOCK_METHOD".into(), "UNLOCK".into()),
            ("TF_HTTP_USERNAME".into(), "shaula-state".into()),
            ("TF_HTTP_PASSWORD".into(), self.capability.expose().into()),
            ("TF_HTTP_RETRY_MAX".into(), "2".into()),
            ("TF_HTTP_RETRY_WAIT_MIN".into(), "1".into()),
            ("TF_HTTP_RETRY_WAIT_MAX".into(), "5".into()),
        ]);
        Ok(env)
    }

    pub(crate) fn install(&self, workspace: &Path) -> CoreResult<()> {
        reject_local_state(workspace)?;
        reject_template_backends(workspace, false)?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        // Do not overwrite a pre-existing reserved file (even identical bytes).
        let mut file = options
            .open(workspace.join(BACKEND_FILE))
            .map_err(|_| invalid())?;
        file.write_all(BACKEND_HCL).map_err(|_| invalid())?;
        file.sync_all().map_err(|_| invalid())?;
        Ok(())
    }

    pub(crate) fn verify(&self, workspace: &Path, initialized: bool) -> CoreResult<()> {
        verify_system_file(workspace)?;
        reject_local_state(workspace)?;
        reject_template_backends(workspace, true)?;
        if initialized {
            let directory = workspace.join(".terraform");
            let path = directory.join("terraform.tfstate");
            let directory_meta = std::fs::symlink_metadata(directory).map_err(|_| invalid())?;
            let meta = std::fs::symlink_metadata(&path).map_err(|_| invalid())?;
            if !directory_meta.is_dir() || !meta.is_file() || meta.len() > 64 * 1024 {
                return Err(invalid());
            }
            let bytes = std::fs::read(path).map_err(|_| invalid())?;
            let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
            // Explicit values in cached backend config take precedence over
            // env defaults. Accept ONLY the empty worker HCL's null config,
            // so tampered metadata cannot redirect a credential-bearing child.
            let config = value.pointer("/backend/config").and_then(|v| v.as_object());
            if value.pointer("/backend/type").and_then(|v| v.as_str()) != Some("http")
                || config.is_none_or(|config| config.values().any(|v| !v.is_null()))
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

/// The ONLY material-digest exception for the new runtime; the legacy digest
/// remains unchanged. No wildcard/prefix exclusion can hide added .tf source.
pub(crate) fn verify_system_file(workspace: &Path) -> CoreResult<()> {
    let path = workspace.join(BACKEND_FILE);
    let metadata = std::fs::symlink_metadata(&path).map_err(|_| invalid())?;
    if !metadata.is_file()
        || metadata.len() != u64::try_from(BACKEND_HCL.len()).map_err(|_| invalid())?
    {
        return Err(invalid());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(invalid());
        }
    }
    if std::fs::read(&path).map_err(|_| invalid())? != BACKEND_HCL {
        return Err(invalid());
    }
    Ok(())
}

pub(crate) fn fresh_workspace(workspace: &Path) -> CoreResult<()> {
    match std::fs::symlink_metadata(workspace) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(meta) if meta.is_dir() => {
            if std::fs::read_dir(workspace)
                .map_err(|_| invalid())?
                .next()
                .is_some()
            {
                return Err(invalid());
            }
            Ok(())
        }
        _ => Err(invalid()),
    }
}

fn reject_local_state(workspace: &Path) -> CoreResult<()> {
    for name in [
        "terraform.tfstate",
        "terraform.tfstate.backup",
        "terraform.tfstate.d",
        "errored.tfstate",
    ] {
        match std::fs::symlink_metadata(workspace.join(name)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(invalid()),
        }
    }
    Ok(())
}

/// Deliberately conservative policy, not a home-grown HCL parser. Native HCL
/// backend/cloud block keywords are rejected even inside comments/strings.
/// JSON keys are decoded first so Unicode escapes cannot hide an override.
fn reject_template_backends(dir: &Path, allow_system: bool) -> CoreResult<()> {
    fn json_backend(value: &serde_json::Value) -> bool {
        match value {
            serde_json::Value::Object(map) => map
                .iter()
                .any(|(k, v)| k == "backend" || k == "cloud" || json_backend(v)),
            serde_json::Value::Array(values) => values.iter().any(json_backend),
            _ => false,
        }
    }
    fn scan(dir: &Path, root: &Path, allow_system: bool) -> CoreResult<()> {
        for entry in std::fs::read_dir(dir).map_err(|_| invalid())? {
            let entry = entry.map_err(|_| invalid())?;
            let path = entry.path();
            if path == root.join(".terraform") {
                continue;
            }
            if path == root.join(BACKEND_FILE) {
                if allow_system {
                    continue;
                }
                return Err(invalid());
            }
            let kind = entry.file_type().map_err(|_| invalid())?;
            if kind.is_dir() {
                scan(&path, root, allow_system)?;
            } else if !kind.is_file() {
                return Err(invalid());
            } else {
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                if name.ends_with(".tf") {
                    let source = std::fs::read_to_string(&path).map_err(|_| invalid())?;
                    if source
                        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                        .any(|word| word == "backend" || word == "cloud")
                    {
                        return Err(invalid());
                    }
                } else if name.ends_with(".tf.json") {
                    let source = std::fs::read(&path).map_err(|_| invalid())?;
                    let value = serde_json::from_slice(&source).map_err(|_| invalid())?;
                    if json_backend(&value) {
                        return Err(invalid());
                    }
                }
            }
        }
        Ok(())
    }
    scan(dir, dir, allow_system)
}

#[cfg(test)]
#[path = "http_backend_tests.rs"]
mod tests;
