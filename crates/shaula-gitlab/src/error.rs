use shaula_core::ports::AccessFailure;

#[derive(Debug, thiserror::Error)]
pub enum GitlabError {
    #[error("configuration error: {0}")]
    Configuration(String),
    #[error("a supported GitLab server version is required")]
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
    #[error("GitLab unavailable: {0}")]
    Unavailable(String),
    #[error("GitLab returned HTTP {status}: {summary}")]
    Http { status: u16, summary: String },
    #[error("invalid GitLab response: {0}")]
    InvalidResponse(String),
    #[error("response exceeds configured bound")]
    ResponseTooLarge,
}

impl GitlabError {
    pub fn to_access_failure(&self) -> AccessFailure {
        match self {
            Self::Unauthenticated => AccessFailure::Unauthenticated,
            Self::PermissionDenied => AccessFailure::PermissionDenied,
            Self::TargetHiddenOrNotFound => AccessFailure::TargetHiddenOrNotFound,
            Self::RateLimited => AccessFailure::RateLimited { retry_after: None },
            Self::RequestUncertain(summary) => AccessFailure::RequestUncertain {
                summary: summary.clone(),
            },
            Self::Http { summary, .. }
            | Self::Unavailable(summary)
            | Self::InvalidResponse(summary) => AccessFailure::Unavailable {
                summary: summary.clone(),
            },
            Self::Configuration(summary) => AccessFailure::Unavailable {
                summary: format!("configuration: {summary}"),
            },
            Self::UnsupportedServerVersion | Self::ResponseTooLarge => AccessFailure::Unavailable {
                summary: self.to_string(),
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

impl From<url::ParseError> for GitlabError {
    fn from(error: url::ParseError) -> Self {
        Self::Configuration(error.to_string())
    }
}
