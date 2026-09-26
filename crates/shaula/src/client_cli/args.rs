use clap::{Args, Subcommand, ValueEnum};
use std::path::PathBuf;
#[derive(Args, Default)]
pub struct ClientArgs {
    #[arg(long, global = true)]
    pub client_config: Option<PathBuf>,
    #[arg(long, global = true, env = "SHAULA_CONTEXT")]
    pub context: Option<String>,
    #[arg(long, global = true, env = "SHAULA_SERVER")]
    pub server: Option<String>,
    #[arg(long, global = true, value_enum)]
    pub credential_kind: Option<CredentialKind>,
    #[arg(long, global = true, conflicts_with = "token_stdin")]
    pub token_file: Option<PathBuf>,
    #[arg(long, global = true, conflicts_with = "token_file")]
    pub token_stdin: bool,
    /// Permit plaintext only to an explicit loopback IP (for an SSH tunnel).
    #[arg(long, global = true)]
    pub allow_loopback_http: bool,
    #[arg(long, global = true, value_enum)]
    pub output: Option<Output>,
}
#[derive(Clone, Copy, ValueEnum, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CredentialKind {
    Pat,
    Oidc,
}
#[derive(Clone, Copy, ValueEnum)]
pub enum Output {
    Table,
    Json,
}

#[derive(Subcommand)]
pub enum RemoteCommand {
    /// Manage Fleet desired state. JSON files use the HTTP API request schema.
    Fleets {
        #[command(subcommand)]
        action: ResourceAction,
    },
    /// Publish templates, inspect input contracts and edit bindings.
    Templates {
        #[command(subcommand)]
        action: ResourceAction,
    },
    /// Manage shared Template Pools.
    Pools {
        #[command(subcommand)]
        action: ResourceAction,
    },
    /// Manage GitHub and Forgejo authentication profiles.
    AuthProfiles {
        #[command(subcommand)]
        action: ResourceAction,
    },
    Jobs {
        #[command(subcommand)]
        action: HistoryAction,
    },
    Generations {
        #[command(subcommand)]
        action: HistoryAction,
    },
    Invocations {
        #[command(subcommand)]
        action: HistoryAction,
    },
    Logs {
        #[command(subcommand)]
        action: LogAction,
    },
    Changes {
        #[command(subcommand)]
        action: ChangeAction,
    },
    Tokens {
        #[command(subcommand)]
        action: TokenAction,
    },
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
    Health {
        #[arg(value_parser=["live","ready"])]
        kind: String,
    },
    Contexts {
        #[arg(value_parser=["list"])]
        action: String,
    },
}
#[derive(Subcommand)]
pub enum ResourceAction {
    Explain {
        key: String,
        #[arg(long)]
        watch: bool,
        #[arg(long, default_value = "300s", value_parser = seconds)]
        timeout: u64,
    },

    List,
    Get {
        key: String,
    },
    Status {
        key: String,
        #[arg(long)]
        watch: bool,
        #[arg(long, default_value = "300s", value_parser = seconds)]
        timeout: u64,
    },
    Create(WriteArgs),
    Update(WriteArgs),
    Publish(WriteArgs),
    Rotate(WriteArgs),
    PolicyUpdate(WriteArgs),
    Retire(WriteArgs),
    Edit {
        key: String,
        #[arg(long)]
        wait: bool,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    Impact {
        key: String,
    },
    InstallationLink {
        key: String,
    },
    Sources {
        #[arg(value_parser=["list"])]
        action: String,
    },
    Variables {
        digest: String,
    },
    Revisions {
        #[command(subcommand)]
        action: RevisionAction,
    },
    InputContract {
        key: String,
        revision: i64,
    },
    Attestations {
        #[arg(value_parser=["get","create","put"])]
        action: String,
        key: String,
        revision: i64,
        attestation: String,
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long)]
        if_match: Option<String>,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    Upload {
        digest: String,
        #[arg(long)]
        file: PathBuf,
    },
    Artifacts {
        #[command(subcommand)]
        action: ArtifactAction,
    },
}
#[derive(Subcommand)]
pub enum RevisionAction {
    Get { key: String, revision: i64 },
    Publish(WriteArgs),
}
#[derive(Subcommand)]
pub enum ArtifactAction {
    Upload {
        digest: String,
        #[arg(long)]
        file: PathBuf,
    },
}
#[derive(Args)]
pub struct WriteArgs {
    pub key: String,
    #[arg(long)]
    pub file: Option<PathBuf>,
    /// Exact quoted version returned when reviewing this resource.
    #[arg(long)]
    pub if_match: Option<String>,
    #[arg(long)]
    pub idempotency_key: Option<String>,
    /// Requires the corresponding read scope; acceptance survives tracking failure.
    #[arg(long)]
    pub wait: bool,
    #[arg(long, default_value = "300s", value_parser = seconds)]
    pub timeout: u64,
    #[arg(long)]
    pub yes: bool,
}
#[derive(Args, Default)]
pub struct Filters {
    #[arg(long)]
    pub fleet: Option<String>,
    #[arg(long)]
    pub status: Option<String>,
    #[arg(long)]
    pub repository: Option<String>,
    #[arg(long)]
    pub job_name: Option<String>,
    #[arg(long)]
    pub association: Option<String>,
    /// UTC Unix milliseconds.
    #[arg(long)]
    pub since: Option<i64>,
    #[arg(long)]
    pub until: Option<i64>,
    #[arg(long)]
    pub cursor: Option<String>,
    #[arg(long)]
    pub limit: Option<u32>,
    #[arg(long)]
    pub all: bool,
    #[arg(long, default_value_t = 1000)]
    pub max_items: usize,
}
#[derive(Subcommand)]
pub enum HistoryAction {
    Explain {
        key: String,
        #[arg(long)]
        watch: bool,
        #[arg(long, default_value = "300s", value_parser = seconds)]
        timeout: u64,
    },

    List {
        id: Option<String>,
        #[command(flatten)]
        filters: Filters,
    },
    Get {
        id: String,
    },
    Finalize {
        id: String,
        #[arg(long)]
        reason: String,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        confirmed_absent: bool,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
}
#[derive(Args)]
pub struct LogArgs {
    pub id: String,
    #[arg(long)]
    pub cursor: Option<String>,
    #[arg(long)]
    pub limit_bytes: Option<u32>,
    #[arg(long,value_parser=["init","plan","apply"])]
    pub phase: Option<String>,
    #[arg(long,value_parser=["stdout","stderr"])]
    pub stream: Option<String>,
    #[arg(long)]
    pub follow: bool,
    #[arg(long, default_value = "300s", value_parser = seconds)]
    pub timeout: u64,
}
#[derive(Subcommand)]
pub enum LogAction {
    Read(LogArgs),
    Download {
        #[command(flatten)]
        args: LogArgs,
        #[arg(long)]
        file: PathBuf,
    },
}
#[derive(Subcommand)]
pub enum ChangeAction {
    Get {
        id: String,
        #[arg(long)]
        kind: String,
    },
    Wait {
        id: String,
        #[arg(long)]
        kind: String,
        #[arg(long, default_value = "300s", value_parser = seconds)]
        timeout: u64,
    },
}
#[derive(Args)]
pub struct IssueArgs {
    #[arg(long)]
    pub name: String,
    #[arg(long = "scope")]
    pub scopes: Vec<String>,
    #[arg(long, default_value = "90d")]
    pub ttl: String,
    #[arg(long)]
    pub idempotency_key: Option<String>,
    #[arg(long, conflicts_with = "show_secret")]
    pub secret_out: Option<PathBuf>,
    #[arg(long)]
    pub show_secret: bool,
}
#[derive(Subcommand)]
pub enum TokenAction {
    List {
        #[arg(long)]
        state: Option<String>,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long)]
        limit: Option<u32>,
    },
    Get {
        id: String,
    },
    Current,
    Create(IssueArgs),
    Revoke {
        id: Option<String>,
        #[arg(long)]
        current: bool,
        #[arg(long)]
        if_match: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    Rotate {
        old_id: String,
        /// Revoke the old token only after the new credential is saved and verified.
        #[arg(long)]
        revoke_old: bool,
        #[command(flatten)]
        issue: IssueArgs,
        #[arg(long)]
        if_match: Option<String>,
        #[arg(long)]
        yes: bool,
    },
}
#[derive(Subcommand)]
pub enum AuthAction {
    Whoami,
    Status,
    /// Validate an externally obtained credential and save it securely.
    Login {
        #[arg(long)]
        credential_out: Option<PathBuf>,
    },
    Logout {
        #[arg(long)]
        revoke: bool,
    },
}

fn seconds(raw: &str) -> Result<u64, String> {
    let (value, multiplier) = if let Some(n) = raw.strip_suffix('m') {
        (n, 60)
    } else {
        (raw.strip_suffix('s').unwrap_or(raw), 1)
    };
    value
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(multiplier))
        .filter(|n| (1..=3600).contains(n))
        .ok_or_else(|| "timeout must be 1s to 3600s".into())
}
