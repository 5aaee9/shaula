//! System-owned Runner setup-log capability; never a Fleet parameter.
use crate::error::{CoreError, CoreResult, ReasonCode};
use serde::{Deserialize, Serialize};

pub const SETUP_INFO_CONTRACT: &str = "shaula.setup-info/v1";

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum SetupInfoDescriptor {
    Disabled,
    Enabled {
        url: String,
        capability: String,
        expires_at: i64,
        wait_seconds: u32,
    },
}

impl std::fmt::Debug for SetupInfoDescriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => f.write_str("SetupInfoDescriptor::Disabled"),
            Self::Enabled {
                expires_at,
                wait_seconds,
                ..
            } => f
                .debug_struct("SetupInfoDescriptor::Enabled")
                .field("url", &"[REDACTED]")
                .field("capability", &"[REDACTED]")
                .field("expires_at", expires_at)
                .field("wait_seconds", wait_seconds)
                .finish(),
        }
    }
}

impl SetupInfoDescriptor {
    /// Builds a descriptor from trusted delivery configuration and a separately issued token.
    pub fn enabled(
        url: String,
        capability: String,
        expires_at: i64,
        wait_seconds: u32,
    ) -> CoreResult<Self> {
        let descriptor = Self::Enabled {
            url,
            capability,
            expires_at,
            wait_seconds,
        };
        descriptor.validate()?;
        Ok(descriptor)
    }

    pub fn validate(&self) -> CoreResult<()> {
        let Self::Enabled {
            url,
            capability,
            expires_at,
            wait_seconds,
        } = self
        else {
            return Ok(());
        };
        let parsed = url::Url::parse(url).map_err(|_| invalid())?;
        if url.len() > 2048
            || url.chars().any(char::is_whitespace)
            || parsed.scheme() != "https"
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || parsed.port() == Some(0)
            || !(32..=512).contains(&capability.len())
            || !capability
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || *expires_at <= 0
            || !(1..=300).contains(wait_seconds)
        {
            return Err(invalid());
        }
        Ok(())
    }

    pub fn validate_for_generation(&self, generation: &str) -> CoreResult<()> {
        self.validate()?;
        if let Self::Enabled { url, .. } = self {
            let parsed = url::Url::parse(url).map_err(|_| invalid())?;
            if parsed.path() != format!("/runner/v1/generations/{generation}/setup-info") {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

fn invalid() -> CoreError {
    CoreError::new(
        ReasonCode::TemplateInvalid,
        "setup-info descriptor rejected",
    )
}

#[cfg(test)]
#[path = "template_setup_info_tests.rs"]
mod tests;
