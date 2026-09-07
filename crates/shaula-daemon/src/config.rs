//! Daemon bootstrap configuration: daemon-native concerns only. Fleet,
//! Template Profile, GitHub Auth Profile or platform-binding catalogs in
//! the bootstrap file fail closed (no second source of truth).

use std::path::PathBuf;

use serde::Deserialize;

/// Bootstrap file shape (`shaula serve --config <path>`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapConfig {
    pub version: u32,
    pub storage: StorageConfig,
    pub http: HttpConfigDto,
    #[serde(default)]
    pub limits: LimitsConfig,
    #[serde(default)]
    pub execution: ExecutionConfig,
    #[serde(default)]
    pub observability: ObservabilityConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageConfig {
    pub data_dir: PathBuf,
    #[serde(default = "default_database")]
    pub database: String,
    #[serde(default = "default_work_root")]
    pub work_root: String,
    #[serde(default = "default_artifact_root")]
    pub artifact_root: String,
}

fn default_database() -> String {
    "shaula.db".to_string()
}

fn default_work_root() -> String {
    "runners".to_string()
}

fn default_artifact_root() -> String {
    "template-artifacts".to_string()
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpConfigDto {
    /// Loopback-only: `127.0.0.1:8080` form.
    pub listen: String,
    #[serde(default = "default_body_limit")]
    pub request_body_limit: String,
    #[serde(default = "default_artifact_limit")]
    pub artifact_body_limit: String,
    /// Exact external principals with explicitly granted resource scopes.
    #[serde(default)]
    pub authorization: Vec<AuthorizationGrant>,
    /// Per-deployment server key for the opaque bindings commitments.
    /// Must be unique per deployment; hardcoded keys break the
    /// non-verifier property of `bindings_digest` (0005 §5).
    pub bindings_server_key: String,
}

impl std::fmt::Debug for HttpConfigDto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpConfigDto")
            .field("listen", &self.listen)
            .field("request_body_limit", &self.request_body_limit)
            .field("artifact_body_limit", &self.artifact_body_limit)
            .field("bindings_server_key", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationGrant {
    pub issuer: String,
    pub subject: String,
    pub scopes: Vec<String>,
}

fn default_body_limit() -> String {
    "1MiB".to_string()
}

fn default_artifact_limit() -> String {
    "64MiB".to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitsConfig {
    #[serde(default = "default_max_fleets")]
    pub max_active_fleets: usize,
    #[serde(default = "default_max_pending")]
    pub max_pending_changes: usize,
}

fn default_max_fleets() -> usize {
    100
}

fn default_max_pending() -> usize {
    1000
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            max_active_fleets: default_max_fleets(),
            max_pending_changes: default_max_pending(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionConfig {
    #[serde(default = "default_create_concurrency")]
    pub create_concurrency: usize,
    #[serde(default = "default_destroy_concurrency")]
    pub destroy_concurrency: usize,
    #[serde(default = "default_operation_timeout_secs")]
    pub operation_timeout_secs: u64,
    pub engines: EnginesConfig,
}

fn default_create_concurrency() -> usize {
    8
}

fn default_destroy_concurrency() -> usize {
    8
}

fn default_operation_timeout_secs() -> u64 {
    1800
}

impl Default for ExecutionConfig {
    // Built from the same default_* authorities as the serde field
    // defaults — an embedded YAML document would add a parse/unwrap path
    // and a second source of truth for values no user ever wrote.
    fn default() -> Self {
        Self {
            create_concurrency: default_create_concurrency(),
            destroy_concurrency: default_destroy_concurrency(),
            operation_timeout_secs: default_operation_timeout_secs(),
            engines: EnginesConfig {
                terraform: EngineConfig {
                    executable: "terraform".to_string(),
                },
            },
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnginesConfig {
    pub terraform: EngineConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineConfig {
    pub executable: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservabilityConfig {
    #[serde(default = "default_service_name")]
    pub service_name: String,
}

impl Default for ObservabilityConfig {
    // A MISSING observability section must fall back to the same default as
    // a missing field: the derived Default would silently log an empty
    // service name for the process lifetime.
    fn default() -> Self {
        Self {
            service_name: default_service_name(),
        }
    }
}

fn default_service_name() -> String {
    "shaula".to_string()
}
