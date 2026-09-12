#[derive(Debug, thiserror::Error)]
pub enum ForgejoError {
    #[error("configuration error: {0}")]
    Configuration(String),
    #[error("a stable Forgejo server version >= 15 is required")]
    UnsupportedServerVersion,
    #[error("authentication failed")]
    Unauthenticated,
    #[error("permission denied")]
    PermissionDenied,
    #[error("target hidden or not found")]
    TargetHiddenOrNotFound,
    #[error("rate limited")]
    RateLimited,
    #[error("request outcome uncertain: {0}")]
    RequestUncertain(String),
    #[error("Forgejo unavailable: {0}")]
    Unavailable(String),
    #[error("Forgejo returned HTTP {status}: {summary}")]
    Http { status: u16, summary: String },
    #[error("invalid Forgejo response: {0}")]
    InvalidResponse(String),
    #[error("response exceeds configured bound")]
    ResponseTooLarge,
}

impl ForgejoError {
    pub fn to_access_failure(&self) -> shaula_core::ports::AccessFailure {
        match self {
            Self::UnsupportedServerVersion => shaula_core::ports::AccessFailure::Unavailable {
                summary: self.to_string(),
            },
            Self::Unauthenticated => shaula_core::ports::AccessFailure::Unauthenticated,
            Self::PermissionDenied => shaula_core::ports::AccessFailure::PermissionDenied,
            Self::TargetHiddenOrNotFound => {
                shaula_core::ports::AccessFailure::TargetHiddenOrNotFound
            }
            Self::RateLimited => {
                shaula_core::ports::AccessFailure::RateLimited { retry_after: None }
            }
            Self::RequestUncertain(summary) => {
                shaula_core::ports::AccessFailure::RequestUncertain {
                    summary: summary.clone(),
                }
            }
            Self::Http { status, summary } => match status {
                401 => shaula_core::ports::AccessFailure::Unauthenticated,
                403 => shaula_core::ports::AccessFailure::PermissionDenied,
                404 => shaula_core::ports::AccessFailure::TargetHiddenOrNotFound,
                429 => shaula_core::ports::AccessFailure::RateLimited { retry_after: None },
                _ => shaula_core::ports::AccessFailure::Unavailable {
                    summary: summary.clone(),
                },
            },
            Self::Unavailable(summary) | Self::InvalidResponse(summary) => {
                shaula_core::ports::AccessFailure::Unavailable {
                    summary: summary.clone(),
                }
            }
            Self::Configuration(summary) => shaula_core::ports::AccessFailure::Unavailable {
                summary: format!("configuration: {summary}"),
            },
            Self::ResponseTooLarge => shaula_core::ports::AccessFailure::Unavailable {
                summary: "response exceeds configured bound".into(),
            },
        }
    }

    pub(crate) fn uncertain(summary: impl Into<String>) -> Self {
        Self::RequestUncertain(summary.into())
    }

    pub(crate) fn unavailable(summary: impl Into<String>) -> Self {
        Self::Unavailable(summary.into())
    }
}

impl From<url::ParseError> for ForgejoError {
    fn from(error: url::ParseError) -> Self {
        Self::Configuration(error.to_string())
    }
}
