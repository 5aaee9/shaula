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

use crate::auth::{app_jwt, transport_error};
use crate::body::BoundedBody;
use crate::client::{classify_response, github_request, retry_after_ms};
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

pub use shaula_core::auth_validation::{
    InstallationLookup, InstallationProof, MetadataReachability, RepoIdentity,
};

/// The `/app` proof of the declared numeric App identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppVerification {
    pub app_id: i64,
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

/// Extracts the numeric App id from a `GET /app` body and proves it is a
/// valid GitHub id equal to the operator-declared App id.
pub(crate) fn declared_app_id(
    app_id: &str,
    body: &serde_json::Value,
) -> Result<i64, ScalesetError> {
    let id = body
        .get("id")
        .and_then(serde_json::Value::as_i64)
        .filter(|id| crate::route_identity::valid_id(*id))
        .ok_or_else(|| ScalesetError::MalformedResponse {
            summary: "app identity response missing numeric id".into(),
        })?;
    if id.to_string() != app_id {
        return Err(ScalesetError::Configuration {
            summary: "authenticated app differs from the declared app id".into(),
        });
    }
    Ok(id)
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
        let response = self.app_request(app_id, private_key, "/app").await?;
        let body: serde_json::Value =
            response
                .bounded_json()
                .await
                .map_err(|_| ScalesetError::MalformedResponse {
                    summary: "app identity response invalid".into(),
                })?;
        let id = declared_app_id(app_id, &body)?;
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
            Ok(response) => response,
            Err(ScalesetError::Status { status: 404, .. }) => {
                return InstallationLookup::NotFound;
            }
            Err(ScalesetError::RateLimited {
                retry_after_secs, ..
            }) => {
                return InstallationLookup::Transient {
                    retry_after_ms: retry_after_ms(retry_after_secs),
                };
            }
            // An ORDINARY 403 is a real permission denial: terminal, not
            // an endless transient (F8).
            Err(ScalesetError::Status { status: 403, .. }) => {
                return InstallationLookup::PermissionDenied;
            }
            Err(_) => {
                return InstallationLookup::Transient {
                    retry_after_ms: None,
                };
            }
        };
        let declared_kind = match selector {
            TargetSelector::AccountRepositories { account_kind, .. } => Some(*account_kind),
            _ => None,
        };
        Self::classify_installation_response(
            response,
            app_id,
            None,
            declared_kind,
            required_permission(selector),
        )
        .await
    }

    pub(crate) async fn classify_installation_response(
        response: reqwest::Response,
        app_id: &str,
        expected_installation_id: Option<i64>,
        declared_kind: Option<AccountKind>,
        (required_name, required_access): (&str, &str),
    ) -> InstallationLookup {
        let parsed: InstallationResponse = match response.bounded_json().await {
            Ok(parsed) => parsed,
            Err(_) => {
                return InstallationLookup::Transient {
                    retry_after_ms: None,
                };
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
        if expected_installation_id.is_some_and(|id| parsed.id != id)
            || parsed.app_id.to_string() != app_id
        {
            return InstallationLookup::IdentityMismatch;
        }
        let account_kind = match parsed.account.account_type.as_str() {
            "User" => AccountKind::User,
            "Organization" => AccountKind::Organization,
            _ => return InstallationLookup::IdentityMismatch,
        };
        if declared_kind.is_some_and(|declared| declared != account_kind) {
            return InstallationLookup::IdentityMismatch;
        }
        if parsed.suspended_at.is_some() {
            return InstallationLookup::Suspended;
        }
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
            has_required_permission: true,
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
        let response = github_request(
            self.http.post(format!(
                "{}/app/installations/{installation_id}/access_tokens",
                self.github_api_base
            )),
            jwt,
        )
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(transport_error)?;
        let response = classify_response(response, "installation token request failed")?;
        #[derive(serde::Deserialize)]
        struct InstallationAccessToken {
            token: String,
        }
        let parsed: InstallationAccessToken =
            response
                .bounded_json()
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

    /// One App-JWT-authenticated GET. Rate-limited `403`/`429` are classified
    /// as [`ScalesetError::RateLimited`]. The fixed User-Agent is attached to
    /// EVERY request, and redirects stay disabled so a hostile origin can never
    /// capture the App JWT.
    pub(crate) async fn app_request(
        &self,
        app_id: &str,
        private_key: &SecretString,
        path: &str,
    ) -> Result<reqwest::Response, ScalesetError> {
        let now = self.clock.now_unix_ms() / 1000;
        let jwt = app_jwt(app_id, private_key.expose(), now)?;
        let response = github_request(
            self.http.get(format!("{}{path}", self.github_api_base)),
            jwt,
        )
        .send()
        .await
        .map_err(transport_error)?;
        classify_response(response, "app request failed")
    }
}
