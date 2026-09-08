//! Typed error classification for the Actions Service wire protocol.
//! Provider bodies stay behind protected diagnostic boundaries: only stable
//! codes and sanitized summaries escape.

use shaula_core::ports::AccessFailure;

/// Typed Actions Service exception names observed in error bodies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionsException {
    AgentExists,
    AgentNotFound,
    JobStillRunning,
    MessageQueueTokenExpired,
    Other,
}

pub fn classify_exception(type_name: &str) -> ActionsException {
    if type_name.contains("AgentExistsException") {
        ActionsException::AgentExists
    } else if type_name.contains("AgentNotFoundException") {
        ActionsException::AgentNotFound
    } else if type_name.contains("JobStillRunningException") {
        ActionsException::JobStillRunning
    } else if type_name.contains("MessageQueueTokenExpiredException") {
        ActionsException::MessageQueueTokenExpired
    } else {
        ActionsException::Other
    }
}

/// An HTTP failure observed while talking to GitHub or the Actions Service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScalesetError {
    /// A new effect was refused before dispatch because route proof failed.
    Authorization { failure: AccessFailure },
    /// Transport-level: request may or may not have been processed.
    RequestUncertain { summary: String },
    /// Typed service exception with bounded context.
    Service {
        exception: ActionsException,
        status: u16,
        summary: String,
    },
    /// Plain status failure without a known exception body.
    Status { status: u16, summary: String },
    /// Primary/secondary rate limiting: always a bounded retry, never a
    /// terminal credential or permission verdict (spec 0011 §4.1).
    RateLimited {
        retry_after_secs: Option<i64>,
        summary: String,
    },
    /// Response body could not be parsed as the expected DTO.
    MalformedResponse { summary: String },
    /// Client-side configuration or construction error.
    Configuration { summary: String },
}

impl ScalesetError {
    pub fn summary(&self) -> String {
        match self {
            ScalesetError::Authorization { failure } => failure.summary(),
            ScalesetError::RequestUncertain { summary } => summary.clone(),
            ScalesetError::Service {
                exception, status, ..
            } => {
                format!("service {exception:?} status={status}")
            }
            ScalesetError::Status { status, summary } => format!("status={status}: {summary}"),
            ScalesetError::RateLimited {
                retry_after_secs,
                summary,
            } => match retry_after_secs {
                Some(secs) => format!("rate limited retry_after={secs}s: {summary}"),
                None => format!("rate limited: {summary}"),
            },
            ScalesetError::MalformedResponse { summary } => summary.clone(),
            ScalesetError::Configuration { summary } => summary.clone(),
        }
    }

    pub fn status(&self) -> Option<u16> {
        match self {
            ScalesetError::Authorization { failure } => match failure {
                AccessFailure::Unauthenticated | AccessFailure::SessionExpired => Some(401),
                AccessFailure::PermissionDenied => Some(403),
                AccessFailure::TargetHiddenOrNotFound => Some(404),
                AccessFailure::RateLimited { .. } => Some(429),
                _ => None,
            },
            ScalesetError::Service { status, .. } | ScalesetError::Status { status, .. } => {
                Some(*status)
            }
            ScalesetError::RateLimited { .. } => Some(403),
            _ => None,
        }
    }

    /// Maps a wire failure onto the core access-failure vocabulary.
    /// `401`/`403`/access-filtered `404` become access failures, never
    /// absence proofs.
    pub fn to_access_failure(&self) -> AccessFailure {
        match self {
            ScalesetError::Authorization { failure } => failure.clone(),
            ScalesetError::RequestUncertain { summary } => AccessFailure::RequestUncertain {
                summary: summary.clone(),
            },
            ScalesetError::RateLimited {
                retry_after_secs, ..
            } => AccessFailure::RateLimited {
                retry_after: retry_after_secs
                    .map(|s| std::time::Duration::from_secs(s.max(0) as u64)),
            },
            ScalesetError::Service { status, .. } | ScalesetError::Status { status, .. } => {
                match *status {
                    401 => AccessFailure::Unauthenticated,
                    403 => AccessFailure::PermissionDenied,
                    404 => AccessFailure::TargetHiddenOrNotFound,
                    429 => AccessFailure::RateLimited { retry_after: None },
                    _ => AccessFailure::Unavailable {
                        summary: self.summary(),
                    },
                }
            }
            ScalesetError::MalformedResponse { summary } => AccessFailure::Unavailable {
                summary: summary.clone(),
            },
            ScalesetError::Configuration { summary } => AccessFailure::Unavailable {
                summary: summary.clone(),
            },
        }
    }
}

impl std::fmt::Display for ScalesetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.summary())
    }
}

impl std::error::Error for ScalesetError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exception_classification() {
        assert_eq!(
            classify_exception("AgentNotFoundException"),
            ActionsException::AgentNotFound
        );
        assert_eq!(
            classify_exception("AgentExistsException"),
            ActionsException::AgentExists
        );
        assert_eq!(
            classify_exception("JobStillRunningException"),
            ActionsException::JobStillRunning
        );
        assert_eq!(
            classify_exception("MessageQueueTokenExpiredException"),
            ActionsException::MessageQueueTokenExpired
        );
        assert_eq!(
            classify_exception("SomethingElseException"),
            ActionsException::Other
        );
    }

    #[test]
    fn access_failure_mapping_never_treats_404_as_absence() {
        let err = ScalesetError::Status {
            status: 404,
            summary: "filtered".into(),
        };
        assert!(matches!(
            err.to_access_failure(),
            AccessFailure::TargetHiddenOrNotFound
        ));
        let err = ScalesetError::Status {
            status: 403,
            summary: "denied".into(),
        };
        assert!(matches!(
            err.to_access_failure(),
            AccessFailure::PermissionDenied
        ));
        let err = ScalesetError::Status {
            status: 401,
            summary: "stale".into(),
        };
        assert!(matches!(
            err.to_access_failure(),
            AccessFailure::Unauthenticated
        ));
    }

    #[test]
    fn summaries_do_not_carry_body_content() {
        let err = ScalesetError::Service {
            exception: ActionsException::JobStillRunning,
            status: 409,
            summary: "internal-only".into(),
        };
        let mapped = err.to_access_failure().summary().to_string();
        assert!(
            !mapped.contains("internal-only"),
            "mapped access failures carry bounded summaries"
        );
    }
}
