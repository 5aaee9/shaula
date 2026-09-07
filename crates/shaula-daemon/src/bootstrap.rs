//! Validated bootstrap and the process-lifetime configuration contract,
//! split to keep files within 400 lines (AGENTS.md).

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::config::BootstrapConfig;

/// Validated bootstrap, frozen for the process lifetime.
#[derive(Clone)]
pub struct ValidatedBootstrap {
    pub data_dir: PathBuf,
    pub database_path: PathBuf,
    pub work_root: PathBuf,
    pub artifact_root: PathBuf,
    pub listen: String,
    pub backend_token: String,
    pub bindings_server_key: String,
    pub request_body_limit: usize,
    pub artifact_body_limit: usize,
    pub max_active_fleets: usize,
    pub max_pending_changes: usize,
    pub create_concurrency: usize,
    pub destroy_concurrency: usize,
    pub operation_timeout: Duration,
    pub terraform_executable: PathBuf,
    pub service_name: String,
}

// Type-bound redaction: the backend token and bindings server key are
// credentials and must never appear through Debug formatting.
impl std::fmt::Debug for ValidatedBootstrap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidatedBootstrap")
            .field("data_dir", &self.data_dir)
            .field("database_path", &self.database_path)
            .field("work_root", &self.work_root)
            .field("artifact_root", &self.artifact_root)
            .field("listen", &self.listen)
            .field("backend_token", &"[REDACTED]")
            .field("bindings_server_key", &"[REDACTED]")
            .field("request_body_limit", &self.request_body_limit)
            .field("artifact_body_limit", &self.artifact_body_limit)
            .field("max_active_fleets", &self.max_active_fleets)
            .field("max_pending_changes", &self.max_pending_changes)
            .field("create_concurrency", &self.create_concurrency)
            .field("destroy_concurrency", &self.destroy_concurrency)
            .field("operation_timeout", &self.operation_timeout)
            .field("terraform_executable", &self.terraform_executable)
            .field("service_name", &self.service_name)
            .finish()
    }
}

/// Runtime storage-containment verification (R9-11, spec 0001 §11.1): the
/// static `..` scan cannot see symlinks/junctions. Every root must resolve
/// canonically inside the locked data dir — two data dirs funnelling into
/// one database can never both pass (single-writer isolation).
pub fn verify_storage_containment(bootstrap: &ValidatedBootstrap) -> Result<(), String> {
    let canonical_data = std::fs::canonicalize(&bootstrap.data_dir)
        .map_err(|e| format!("data dir {} unusable: {e}", bootstrap.data_dir.display()))?;
    for (name, dir) in [
        ("work_root", &bootstrap.work_root),
        ("artifact_root", &bootstrap.artifact_root),
    ] {
        let canonical = std::fs::canonicalize(dir)
            .map_err(|e| format!("{name} {} unusable: {e}", dir.display()))?;
        if !canonical.starts_with(&canonical_data) || canonical == canonical_data {
            return Err(format!(
                "{name} {} resolves outside the data directory (symlink or junction redirection is not permitted)",
                dir.display()
            ));
        }
    }
    // The database must sit DIRECTLY inside the data dir: the ownership
    // lock file lives next to it, so a redirected database file would
    // couple two independently locked data dirs to one ledger.
    let db_parent = bootstrap
        .database_path
        .parent()
        .ok_or_else(|| "database path has no parent".to_string())?;
    let canonical_parent = std::fs::canonicalize(db_parent)
        .map_err(|e| format!("database dir {} unusable: {e}", db_parent.display()))?;
    if canonical_parent != canonical_data {
        return Err(format!(
            "database {} resolves outside the data directory; single-writer isolation requires the ledger inside the locked data dir",
            bootstrap.database_path.display()
        ));
    }
    if bootstrap.database_path.is_file() {
        let canonical_db = std::fs::canonicalize(&bootstrap.database_path).map_err(|e| {
            format!(
                "database {} unusable: {e}",
                bootstrap.database_path.display()
            )
        })?;
        if !canonical_db.starts_with(&canonical_data) {
            return Err(format!(
                "database {} is a symlink outside the data directory",
                bootstrap.database_path.display()
            ));
        }
    }
    Ok(())
}

fn parse_size(text: &str) -> Result<usize, String> {
    let text = text.trim();
    let (digits, multiplier) = if let Some(stripped) = text.strip_suffix("MiB") {
        (stripped, 1024usize * 1024)
    } else if let Some(stripped) = text.strip_suffix("KiB") {
        (stripped, 1024)
    } else if let Some(stripped) = text.strip_suffix('B') {
        (stripped, 1)
    } else {
        (text, 1)
    };
    digits
        .trim()
        .parse::<usize>()
        .map(|n| n * multiplier)
        .map_err(|_| format!("invalid size {text:?}"))
}

impl ValidatedBootstrap {
    /// Loads, parses and statically validates bootstrap before the API
    /// becomes ready.
    pub fn load(path: &Path) -> Result<Self, String> {
        let raw =
            std::fs::read_to_string(path).map_err(|e| format!("bootstrap unreadable: {e}"))?;
        let config: BootstrapConfig =
            serde_yaml::from_str(&raw).map_err(|e| format!("bootstrap invalid: {e}"))?;
        Self::validate(config)
    }

    pub fn validate(config: BootstrapConfig) -> Result<Self, String> {
        if config.version != 1 {
            return Err(format!("unsupported bootstrap version {}", config.version));
        }

        let data_dir = config.storage.data_dir;
        let database_path = data_dir.join(&config.storage.database);
        let work_root = data_dir.join(&config.storage.work_root);
        let artifact_root = data_dir.join(&config.storage.artifact_root);

        // Paths must resolve inside the data dir; traversal is rejected
        // textually here, and containment is re-checked after directory
        // creation at startup (the data dir may not exist yet).
        for path in [&work_root, &artifact_root] {
            let rendered = path.to_string_lossy().replace('\\', "/");
            if rendered.split('/').any(|seg| seg == "..") {
                return Err(format!(
                    "storage path {rendered} escapes the data directory"
                ));
            }
        }

        // HTTP listener must be loopback (ADR-0011).
        let listen_parts: Vec<&str> = config
            .http
            .listen
            .rsplit_once(':')
            .map(|(h, p)| vec![h, p])
            .unwrap_or_default();
        if listen_parts.len() != 2 {
            return Err(format!("listen {:?} malformed", config.http.listen));
        }
        shaula_core::net::verify_loopback(listen_parts[0])?;

        let backend_token = config.http.backend_token;
        if backend_token.len() < 16 {
            return Err("http.backend_token must be at least 16 characters".to_string());
        }
        let bindings_server_key = config.http.bindings_server_key;
        if bindings_server_key.len() < 32 {
            return Err("http.bindings_server_key must be at least 32 characters".to_string());
        }

        let request_body_limit = parse_size(&config.http.request_body_limit)?;
        let artifact_body_limit = parse_size(&config.http.artifact_body_limit)?;
        if request_body_limit == 0 || artifact_body_limit == 0 {
            return Err("body limits must be positive".to_string());
        }

        if config.execution.create_concurrency == 0 || config.execution.destroy_concurrency == 0 {
            return Err("concurrency values must be positive".to_string());
        }

        // OpenTofu is not advertised; only terraform is accepted. The
        // path is frozen to ONE absolute executable here so hashing
        // (attestation authority) and spawning share a single engine
        // identity (R5-08) — a bare "terraform" name is resolved
        // against PATH now, not per-use.
        let terraform_executable =
            resolve_engine_executable(&config.execution.engines.terraform.executable)?;

        Ok(Self {
            data_dir,
            database_path,
            work_root,
            artifact_root,
            listen: config.http.listen,
            backend_token,
            bindings_server_key,
            request_body_limit,
            artifact_body_limit,
            max_active_fleets: config.limits.max_active_fleets,
            max_pending_changes: config.limits.max_pending_changes,
            create_concurrency: config.execution.create_concurrency,
            destroy_concurrency: config.execution.destroy_concurrency,
            operation_timeout: Duration::from_secs(config.execution.operation_timeout_secs),
            terraform_executable,
            service_name: config.observability.service_name,
        })
    }
}

/// Freezes the engine executable to ONE STABLE ABSOLUTE path (R5-08,
/// R6-09; spec 0005 §5.152). A bare name is resolved against PATH (with
/// the Windows `.exe` suffix) at bootstrap time; an absolute or
/// multi-component path must already exist and is normalized against
/// the CURRENT cwd — later spawns run from a per-operation workspace,
/// where the original relative spelling would no longer resolve to the
/// verified file. A non-resolvable engine fails bootstrap — fail
/// closed, the daemon cannot attest or apply without its engine.
fn resolve_engine_executable(configured: &str) -> Result<PathBuf, String> {
    let candidate = Path::new(configured);
    if candidate.is_absolute() || candidate.components().count() > 1 {
        let frozen = std::path::absolute(candidate)
            .map_err(|e| format!("engine executable {configured:?} unusable: {e}"))?;
        if !frozen.is_file() {
            return Err(format!(
                "engine executable {} does not exist",
                frozen.display()
            ));
        }
        return Ok(frozen);
    }
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path_var) {
        for attempt in [dir.join(candidate), dir.join(format!("{configured}.exe"))] {
            if attempt.is_file() {
                return std::path::absolute(&attempt)
                    .map_err(|e| format!("engine executable {configured:?} unusable: {e}"));
            }
        }
    }
    Err(format!(
        "engine executable {configured:?} not found on PATH; configure an absolute path"
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A REAL engine file on disk: bootstrap freezes the executable to
    /// an existing absolute path, so tests must point at one (R5-08).
    fn engine_fixture() -> String {
        let dir = std::env::temp_dir().join("shaula-bootstrap-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("terraform-fixture.exe");
        std::fs::write(&path, b"fixture engine").unwrap();
        path.to_string_lossy().replace('\\', "/")
    }

    fn base_config() -> BootstrapConfig {
        serde_yaml::from_str(&format!(
            r#"
version: 1
storage:
  data_dir: /var/lib/shaula
http:
  listen: 127.0.0.1:8080
  backend_token: "bootstrap-backend-token-0123456789"
  bindings_server_key: "bootstrap-bindings-key-0123456789abcdef"
execution:
  engines:
    terraform:
      executable: "{}"
"#,
            engine_fixture()
        ))
        .unwrap()
    }

    #[test]
    fn valid_bootstrap_freezes_absolute_engine_path() {
        let validated = ValidatedBootstrap::validate(base_config()).unwrap();
        assert_eq!(validated.listen, "127.0.0.1:8080");
        assert_eq!(
            validated.database_path,
            PathBuf::from("/var/lib/shaula/shaula.db")
        );
        assert_eq!(validated.max_active_fleets, 100);
        // The frozen authority is the ABSOLUTE fixture path (R5-08).
        assert!(validated.terraform_executable.is_absolute());
        assert!(validated.terraform_executable.is_file());
    }

    #[test]
    fn unresolvable_engine_fails_bootstrap() {
        let mut config = base_config();
        config.execution.engines.terraform.executable = "definitely-not-on-path-xyz".to_string();
        let error = ValidatedBootstrap::validate(config).unwrap_err();
        assert!(
            error.contains("not found on PATH"),
            "a PATH-only engine that does not exist must fail bootstrap: {error}"
        );
        let mut config = base_config();
        config.execution.engines.terraform.executable = "/nonexistent/engine/terraform".to_string();
        assert!(ValidatedBootstrap::validate(config).is_err());
    }

    #[test]
    fn relative_engine_path_is_frozen_absolute() {
        // R6-09: a multi-component RELATIVE path resolves against THIS
        // cwd at bootstrap; the frozen value must be its stable
        // absolute form, because the runtime later spawns from a
        // per-operation workspace where the relative spelling would no
        // longer resolve to the verified file. The fixture lives under
        // target/ so the relative spelling really does resolve from the
        // test process cwd.
        let dir = std::path::Path::new("target/test-relative-engine");
        std::fs::create_dir_all(dir).unwrap();
        let engine = dir.join("terraform.cmd");
        std::fs::write(&engine, b"@echo off\r\nexit /b 0\r\n").unwrap();
        let mut config = base_config();
        config.execution.engines.terraform.executable =
            "target/test-relative-engine/terraform.cmd".to_string();
        let validated = ValidatedBootstrap::validate(config).unwrap();
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(
            validated.terraform_executable,
            cwd.join(&engine),
            "a relative engine path must be frozen to its stable absolute form"
        );
        std::fs::remove_file(&engine).ok();
        std::fs::remove_dir(dir).ok();
    }

    #[test]
    fn non_loopback_listen_fails_closed() {
        let mut config = base_config();
        config.http.listen = "0.0.0.0:8080".to_string();
        assert!(ValidatedBootstrap::validate(config).is_err());
    }

    #[test]
    fn missing_observability_section_defaults_service_name() {
        let config = base_config();
        assert_eq!(config.observability.service_name, "shaula");
    }

    #[test]
    fn unknown_fields_rejected() {
        let raw = r#"
version: 1
storage:
  data_dir: /tmp
http:
  listen: 127.0.0.1:8080
  backend_token: "bootstrap-backend-token"
  bindings_server_key: "bootstrap-bindings-key-0123456789abcdef"
template_profiles:
  - key: kubernetes
"#;
        let parsed: Result<BootstrapConfig, _> = serde_yaml::from_str(raw);
        assert!(
            parsed.is_err(),
            "bootstrap must not carry resource catalogs"
        );
    }

    #[test]
    fn traversal_paths_rejected() {
        let mut config = base_config();
        config.storage.work_root = "../escape".to_string();
        assert!(ValidatedBootstrap::validate(config).is_err());
    }

    #[test]
    fn weak_backend_token_rejected() {
        let mut config = base_config();
        config.http.backend_token = "short".to_string();
        assert!(ValidatedBootstrap::validate(config).is_err());
    }

    #[test]
    fn size_parsing() {
        assert_eq!(parse_size("1MiB"), Ok(1024 * 1024));
        assert_eq!(parse_size("64MiB"), Ok(64 * 1024 * 1024));
        assert_eq!(parse_size("512KiB"), Ok(512 * 1024));
        assert_eq!(parse_size("1024B"), Ok(1024));
        assert!(parse_size("12GB").is_err());
    }
}
