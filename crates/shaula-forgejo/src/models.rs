use serde::{Deserialize, Deserializer, Serialize};
use shaula_core::secret::SecretString;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForgejoScope {
    Instance,
    Organization(String),
    User,
    Repository { owner: String, name: String },
}

impl ForgejoScope {
    /// Returns the Forgejo API route for this scope.
    pub fn api_path(&self) -> String {
        match self {
            Self::Instance => "/api/v1/admin/actions/runners".into(),
            Self::Organization(org) => {
                format!("/api/v1/orgs/{}/actions/runners", encode_path_segment(org))
            }
            Self::User => "/api/v1/user/actions/runners".into(),
            Self::Repository { owner, name } => format!(
                "/api/v1/repos/{}/{}/actions/runners",
                encode_path_segment(owner),
                encode_path_segment(name)
            ),
        }
    }

    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Instance | Self::User => Ok(()),
            Self::Organization(org) => validate_path_segment(org, "organization"),
            Self::Repository { owner, name } => {
                validate_path_segment(owner, "repository owner")?;
                validate_path_segment(name, "repository name")
            }
        }
    }
}

fn validate_path_segment(value: &str, kind: &str) -> Result<(), &'static str> {
    if value.trim().is_empty() {
        return Err(match kind {
            "organization" => "organization must not be empty",
            "repository owner" => "repository owner must not be empty",
            _ => "repository name must not be empty",
        });
    }
    if value.len() > 255 {
        return Err(match kind {
            "organization" => "organization is too long",
            "repository owner" => "repository owner is too long",
            _ => "repository name is too long",
        });
    }
    if matches!(value, "." | "..") || value.chars().any(char::is_control) {
        return Err("scope contains an invalid path segment");
    }
    Ok(())
}

fn encode_path_segment(value: &str) -> String {
    // `form_urlencoded` is available through the already pinned `url` crate.
    // Its only path-incompatible escape is `+` for spaces.
    url::form_urlencoded::byte_serialize(value.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Label {
    pub name: String,
    #[serde(default)]
    pub r#type: String,
}

impl<'de> Deserialize<'de> for Label {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum WireLabel {
            Name(String),
            Detailed {
                name: String,
                #[serde(default)]
                r#type: String,
            },
        }

        match WireLabel::deserialize(deserializer)? {
            WireLabel::Name(name) => Ok(Self {
                name,
                r#type: String::new(),
            }),
            WireLabel::Detailed { name, r#type } => Ok(Self { name, r#type }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Runner {
    pub id: u64,
    pub uuid: String,
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub owner_id: u64,
    #[serde(default)]
    pub repo_id: u64,
    #[serde(default)]
    pub description: String,
    #[serde(default, deserialize_with = "labels_or_empty")]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub ephemeral: bool,
    #[serde(default)]
    pub version: Option<String>,
}

fn labels_or_empty<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Label>, D::Error> {
    Ok(Option::<Vec<Label>>::deserialize(deserializer)?.unwrap_or_default())
}

impl Runner {
    pub fn status_kind(&self) -> RunnerStatus {
        match self.status.as_str() {
            "offline" => RunnerStatus::Offline,
            "idle" => RunnerStatus::Idle,
            "active" => RunnerStatus::Active,
            other => RunnerStatus::Unknown(other.to_string()),
        }
    }

    pub fn is_idle(&self) -> bool {
        matches!(self.status_kind(), RunnerStatus::Idle)
    }

    pub fn is_active(&self) -> bool {
        matches!(self.status_kind(), RunnerStatus::Active)
    }

    pub fn is_known(&self) -> bool {
        !matches!(self.status_kind(), RunnerStatus::Unknown(_))
    }

    pub fn has_labels(&self, labels: &[String]) -> bool {
        labels
            .iter()
            .all(|label| self.labels.iter().any(|candidate| candidate.name == *label))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerStatus {
    Offline,
    Idle,
    Active,
    Unknown(String),
}

impl fmt::Display for RunnerStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Offline => f.write_str("offline"),
            Self::Idle => f.write_str("idle"),
            Self::Active => f.write_str("active"),
            Self::Unknown(status) => f.write_str(status),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub id: u64,
    #[serde(default)]
    pub handle: String,
    #[serde(default)]
    pub attempt: u64,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub runs_on: Vec<String>,
    #[serde(default)]
    pub task_id: u64,
    #[serde(default)]
    pub run_id: u64,
    #[serde(default)]
    pub repo_id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registration {
    pub id: u64,
    pub uuid: String,
    pub token: SecretString,
}

impl Registration {
    pub(crate) fn new(id: u64, uuid: String, token: String) -> Self {
        Self {
            id,
            uuid,
            token: SecretString::new(token),
        }
    }

    pub fn token(&self) -> &str {
        self.token.expose()
    }
}

/// One-shot credential material consumed by a Forgejo runner bootstrap.
///
/// The token is deliberately kept behind [`SecretString`] and is omitted from
/// `Debug`; callers must write it to a protected file rather than placing it
/// in Terraform variables, argv, or resource metadata.
#[derive(Clone, PartialEq, Eq)]
pub struct RunnerBootstrapMaterial {
    pub instance_url: url::Url,
    pub uuid: String,
    pub token: SecretString,
    pub labels: Vec<String>,
}

impl RunnerBootstrapMaterial {
    pub fn into_core(
        self,
    ) -> Result<shaula_core::ports::forgejo::ForgejoBootstrapMaterial, &'static str> {
        shaula_core::ports::forgejo::ForgejoBootstrapMaterial::new(
            self.instance_url.to_string(),
            self.uuid,
            self.token,
            self.labels,
        )
    }
}

impl fmt::Debug for RunnerBootstrapMaterial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunnerBootstrapMaterial")
            .field("instance_url", &self.instance_url)
            .field("uuid", &self.uuid)
            .field("token", &"REDACTED")
            .field("labels", &self.labels)
            .finish()
    }
}

impl RunnerBootstrapMaterial {
    pub fn token(&self) -> &str {
        self.token.expose()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationUncertainty {
    None,
    ExactlyOneCleanupRequired,
    Quarantined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removal {
    Removed,
    AlreadyAbsent,
    Busy,
}
