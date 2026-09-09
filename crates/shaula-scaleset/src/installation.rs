//! Target-aware installation resolution (spec 0011 §4.1/§4.2): an App JWT
//! discovers the installation behind a concrete account or repository,
//! verifies App/account identity, suspension and the REQUIRED permission
//! for the selector's runner scope. Discovery NEVER trusts "any
//! installation of the App": every binding is proven per account and
//! frozen only by promotion.

use std::sync::Arc;

use serde::Deserialize;
use shaula_core::auth_context::RepositorySelection;
use shaula_core::auth_policy::{AccountKind, TargetSelector};
use shaula_core::ports::Clock;
use shaula_core::secret::SecretString;

use crate::auth::{app_jwt, transport_error_pub};
use crate::client::{classify_response, DEFAULT_REQUEST_TIMEOUT, USER_AGENT};
use crate::error::ScalesetError;

/// The required installation permission for ONE selector, classified by
/// the runner scope the selector admits (spec 0011 §4.1 step 3):
/// organization Fleet runners need `organization_self_hosted_runners`,
/// repository Fleet runners need repository `administration`.
pub fn required_permission_for_target(
    target: &shaula_core::github::GitHubTarget,
) -> (&'static str, &'static str) {
    match target {
        shaula_core::github::GitHubTarget::Organization { .. } => {
            ("organization_self_hosted_runners", "write")
        }
        shaula_core::github::GitHubTarget::Repository { .. } => ("administration", "write"),
    }
}

pub fn required_permission(selector: &TargetSelector) -> (&'static str, &'static str) {
    match selector {
        TargetSelector::Organization { .. } => ("organization_self_hosted_runners", "write"),
        TargetSelector::Repository { .. } | TargetSelector::AccountRepositories { .. } => {
            ("administration", "write")
        }
    }
}

/// Verified installation facts for one account (non-secret).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallationProof {
    pub installation_id: i64,
    pub app_id: i64,
    pub account_id: i64,
    pub account_kind: AccountKind,
    /// Canonical login as GitHub returned it (display only).
    pub login: String,
    pub repository_selection: RepositorySelection,
    pub has_required_permission: bool,
}

/// Classified lookup outcome; the validator maps every variant onto a
/// durable Candidate outcome or a bounded retry (spec 0011 §4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallationLookup {
    Proven(InstallationProof),
    /// 404: no installation for this account/repository.
    NotFound,
    /// The installation exists but is suspended.
    Suspended,
    /// The installation answers for a DIFFERENT App or account than the
    /// declared/selector identity.
    IdentityMismatch,
    /// A required permission is missing from the installation.
    PermissionDenied,
    /// Network failure, `429`, rate-limit `403` or GitHub `5xx`: bounded
    /// retry, never a terminal rejection.
    /// Bounded retry with the deadline GitHub supplied (if any).
    Transient {
        retry_after_ms: Option<i64>,
    },
}

/// Outcome of the bounded installation-metadata probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataReachability {
    Reachable,
    NotFound,
    PermissionDenied,
    Transient { retry_after_ms: Option<i64> },
}

/// The `/app` proof of the declared numeric App identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppVerification {
    pub app_id: i64,
}

/// Classified outcome of one repository identity lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoIdentity {
    /// Durable numeric identity (repository id, owner id).
    Proven(i64, i64),
    /// 404: removed from the installation, renamed or deleted.
    Missing,
    Transient {
        retry_after_ms: Option<i64>,
    },
}

#[derive(Deserialize)]
pub(crate) struct InstallationAccount {
    pub(crate) id: i64,
    pub(crate) login: String,
    #[serde(rename = "type")]
    pub(crate) account_type: String,
}

#[derive(Deserialize)]
pub(crate) struct InstallationResponse {
    pub(crate) id: i64,
    pub(crate) app_id: i64,
    pub(crate) account: InstallationAccount,
    #[serde(default)]
    pub(crate) suspended_at: Option<String>,
    #[serde(default)]
    pub(crate) repository_selection: Option<String>,
    #[serde(default)]
    pub(crate) permissions: std::collections::BTreeMap<String, String>,
}

/// Resolves installations for an exact App credential. All API hosts are
/// derived from the fixed `github.com` configuration; no caller URL is
/// ever used as an endpoint, and credential-bearing requests never follow
/// redirects.
pub struct AppInstallationResolver {
    pub(crate) http: reqwest::Client,
    pub(crate) github_api_base: String,
    pub(crate) clock: Arc<dyn Clock>,
}

impl AppInstallationResolver {
    pub fn new(github_api_base: String, http: reqwest::Client, clock: Arc<dyn Clock>) -> Self {
        Self {
            http,
            github_api_base: github_api_base.trim_end_matches('/').to_string(),
            clock,
        }
    }

    /// Proves the private key authenticates as the DECLARED App
    /// (`GET /app`): the returned numeric id must equal the App id the
    /// operator declared.
    pub async fn verify_app(
        &self,
        app_id: &str,
        private_key: &SecretString,
    ) -> Result<AppVerification, ScalesetError> {
        let response = match self.app_request(app_id, private_key, "/app").await? {
            Ok(response) => response,
            Err(status_error) => return Err(status_error),
        };
        let body: serde_json::Value =
            response
                .json()
                .await
                .map_err(|_| ScalesetError::MalformedResponse {
                    summary: "app identity response invalid".into(),
                })?;
        let id = body.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
        if !crate::route_identity::valid_id(id) {
            return Err(ScalesetError::MalformedResponse {
                summary: "app identity response missing numeric id".into(),
            });
        }
        if id.to_string() != app_id {
            return Err(ScalesetError::Configuration {
                summary: "authenticated app differs from the declared app id".into(),
            });
        }
        Ok(AppVerification { app_id: id })
    }

    /// Resolves and classifies the installation behind one selector via
    /// `GET /orgs/{org}/installation`, `GET /users/{user}/installation`
    /// or `GET /repos/{owner}/{repo}/installation`. Account-scoped
    /// selectors dispatch by their DECLARED account kind: organizations
    /// use the org discovery endpoint, personal accounts the user one.
    pub async fn installation_for_selector(
        &self,
        app_id: &str,
        private_key: &SecretString,
        selector: &TargetSelector,
    ) -> InstallationLookup {
        let path = match selector {
            TargetSelector::Organization { owner } => format!("/orgs/{owner}/installation"),
            TargetSelector::AccountRepositories {
                account_kind: AccountKind::Organization,
                owner,
            } => format!("/orgs/{owner}/installation"),
            TargetSelector::AccountRepositories {
                account_kind: AccountKind::User,
                owner,
            } => format!("/users/{owner}/installation"),
            TargetSelector::Repository { owner, repository } => {
                format!("/repos/{owner}/{repository}/installation")
            }
        };
        let response = match self.app_request(app_id, private_key, &path).await {
            Ok(Ok(response)) => response,
            Ok(Err(ScalesetError::Status { status: 404, .. })) => {
                return InstallationLookup::NotFound
            }
            Ok(Err(ScalesetError::RateLimited {
                retry_after_secs, ..
            })) => {
                return InstallationLookup::Transient {
                    retry_after_ms: retry_after_secs.map(|s| s.max(0) * 1000),
                }
            }
            // An ORDINARY 403 is a real permission denial: terminal, not
            // an endless transient (F8).
            Ok(Err(ScalesetError::Status { status: 403, .. })) => {
                return InstallationLookup::PermissionDenied
            }
            Ok(Err(_)) => {
                return InstallationLookup::Transient {
                    retry_after_ms: None,
                }
            }
            Err(_) => {
                return InstallationLookup::Transient {
                    retry_after_ms: None,
                }
            }
        };
        let parsed: InstallationResponse = match response.json().await {
            Ok(parsed) => parsed,
            Err(_) => {
                return InstallationLookup::Transient {
                    retry_after_ms: None,
                }
            }
        };
        if !crate::route_identity::valid_id(parsed.id)
            || !crate::route_identity::valid_id(parsed.app_id)
            || !crate::route_identity::valid_id(parsed.account.id)
        {
            return InstallationLookup::Transient {
                retry_after_ms: None,
            };
        }
        if parsed.app_id.to_string() != app_id {
            // The installation answers for a different App: never a
            // fallback route (spec 0011 §4.2 step 2).
            return InstallationLookup::IdentityMismatch;
        }
        let account_kind = match parsed.account.account_type.as_str() {
            "User" => AccountKind::User,
            "Organization" => AccountKind::Organization,
            _ => return InstallationLookup::IdentityMismatch,
        };
        // A selector's declared account kind must match GitHub's actual
        // account type (spec 0011 §3: no kind mismatch).
        if let TargetSelector::AccountRepositories {
            account_kind: declared,
            ..
        } = selector
        {
            if *declared != account_kind {
                return InstallationLookup::IdentityMismatch;
            }
        }
        if parsed.suspended_at.is_some() {
            return InstallationLookup::Suspended;
        }
        let (required_name, required_access) = required_permission(selector);
        let has_required_permission = parsed
            .permissions
            .get(required_name)
            .is_some_and(|granted| granted == required_access || granted == "admin");
        if !has_required_permission {
            return InstallationLookup::PermissionDenied;
        }
        let repository_selection = match parsed.repository_selection.as_deref() {
            Some("selected") => RepositorySelection::Selected,
            _ => RepositorySelection::All,
        };
        InstallationLookup::Proven(InstallationProof {
            installation_id: parsed.id,
            app_id: parsed.app_id,
            account_id: parsed.account.id,
            account_kind,
            login: parsed.account.login,
            repository_selection,
            has_required_permission,
        })
    }

    pub(crate) async fn mint_installation_token(
        &self,
        app_id: &str,
        private_key: &SecretString,
        installation_id: i64,
    ) -> Result<String, ScalesetError> {
        let now = self.clock.now_unix_ms() / 1000;
        let jwt = app_jwt(app_id, private_key.expose(), now)?;
        let response = self
            .http
            .post(format!(
                "{}/app/installations/{installation_id}/access_tokens",
                self.github_api_base
            ))
            .bearer_auth(jwt)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", USER_AGENT)
            .json(&serde_json::json!({}))
            .timeout(DEFAULT_REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(transport_error_pub)?;
        let response = classify_response(response, "installation token request failed")?;
        #[derive(serde::Deserialize)]
        struct InstallationAccessToken {
            token: String,
        }
        let parsed: InstallationAccessToken =
            response
                .json()
                .await
                .map_err(|_| ScalesetError::MalformedResponse {
                    summary: "installation token body invalid".into(),
                })?;
        if parsed.token.is_empty() {
            return Err(ScalesetError::MalformedResponse {
                summary: "installation token missing".into(),
            });
        }
        Ok(parsed.token)
    }

    /// One App-JWT-authenticated GET. `Ok(Err(status))` reports the HTTP
    /// failure for caller-side classification (rate-limited `403`/`429`
    /// already classified as [`ScalesetError::RateLimited`]); `Err` is
    /// transport. The fixed User-Agent is attached to EVERY request
    /// (GitHub refuses unattributed clients), and redirects stay disabled
    /// so a hostile origin can never capture the App JWT.
    pub(crate) async fn app_request(
        &self,
        app_id: &str,
        private_key: &SecretString,
        path: &str,
    ) -> Result<Result<reqwest::Response, ScalesetError>, ScalesetError> {
        let now = self.clock.now_unix_ms() / 1000;
        let jwt = app_jwt(app_id, private_key.expose(), now)?;
        let response = self
            .http
            .get(format!("{}{path}", self.github_api_base))
            .bearer_auth(jwt)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", USER_AGENT)
            .timeout(DEFAULT_REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(transport_error_pub)?;
        Ok(classify_response(response, "app request failed"))
    }
}
