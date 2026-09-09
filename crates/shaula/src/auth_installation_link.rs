//! Protected, read-only App installation navigation assembled at the root.

use std::sync::Arc;

use shaula_core::auth::AuthProfileKey;
use shaula_core::ports::Clock;
use shaula_core::registry::{Actor, ControlPlaneStore, ProfileHead, Scope};
use shaula_core::secret::SecretString;
use shaula_http::router::auth_installation_link::{
    AuthInstallationLink, AuthInstallationLinkError as Error, AuthInstallationLinkPort,
};
use shaula_scaleset::{AppInstallationResolver, ScalesetError};

pub(crate) struct StoredAuthInstallationLink {
    store: Arc<dyn ControlPlaneStore>,
    resolver: AppInstallationResolver,
}

impl StoredAuthInstallationLink {
    pub(crate) fn production(
        store: Arc<dyn ControlPlaneStore>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, ScalesetError> {
        let http = shaula_scaleset::client::production_http_client()?;
        Ok(Self {
            store,
            resolver: AppInstallationResolver::new(
                shaula_scaleset::config::GITHUB_COM_API_BASE.into(),
                http,
                clock,
            ),
        })
    }

    async fn active_head(&self, key: &str) -> Result<ProfileHead, Error> {
        let head = self
            .store
            .auth_profile_get(key)
            .await
            .map_err(|_| Error::LookupUnavailable)?
            .ok_or(Error::NotFound)?;
        // A pending rotation retains its active authority. Retirement and
        // unknown/legacy states can never supply a navigation credential.
        if !matches!(head.status.as_str(), "Active" | "Validating")
            || head.active_revision.is_none_or(|revision| revision <= 0)
        {
            return Err(Error::ProfileUnavailable);
        }
        Ok(head)
    }
}

#[async_trait::async_trait]
impl AuthInstallationLinkPort for StoredAuthInstallationLink {
    async fn installation_link(
        &self,
        actor: &Actor,
        key: &str,
    ) -> Result<AuthInstallationLink, Error> {
        if !actor.has(Scope::AuthRead) || !actor.has(Scope::AuthWrite) {
            return Err(Error::ScopeDenied);
        }
        AuthProfileKey::new(key).map_err(|_| Error::NotFound)?;
        let head = self.active_head(key).await?;
        let revision = head.active_revision.ok_or(Error::ProfileUnavailable)?;
        let row = self
            .store
            .auth_revision_get(key, revision)
            .await
            .map_err(|_| Error::LookupUnavailable)?
            .ok_or(Error::ProfileUnavailable)?;
        if row.schema_version != 2 || row.kind != "github_app" || row.state != "Active" {
            return Err(Error::ProfileUnavailable);
        }
        let app_id = row.app_id.ok_or(Error::ProfileUnavailable)?;
        let bytes = self
            .store
            .auth_credential_bytes(key, revision)
            .await
            .map_err(|_| Error::LookupUnavailable)?
            .filter(|bytes| !bytes.is_empty())
            .ok_or(Error::LookupUnavailable)?;
        let private_key =
            SecretString::new(String::from_utf8(bytes).map_err(|_| Error::InvalidApp)?);
        let url = self
            .resolver
            .installation_url(&app_id, &private_key)
            .await
            .map_err(link_error)?;
        // The external request yields: never return a link for a retired,
        // replaced or newly activated identity that the UI did not review.
        let current = self.active_head(key).await.map_err(|error| match error {
            Error::LookupUnavailable => error,
            _ => Error::ProfileChanged,
        })?;
        if current.incarnation != head.incarnation || current.active_revision != Some(revision) {
            return Err(Error::ProfileChanged);
        }
        Ok(AuthInstallationLink {
            url,
            app_id,
            revision,
            incarnation: head.incarnation,
        })
    }
}

fn link_error(error: ScalesetError) -> Error {
    match error {
        ScalesetError::Configuration { .. } | ScalesetError::MalformedResponse { .. } => {
            Error::InvalidApp
        }
        ScalesetError::Status {
            status: 300..=499, ..
        } => Error::InvalidApp,
        _ => Error::LookupUnavailable,
    }
}

#[cfg(test)]
#[path = "auth_installation_link_tests.rs"]
mod tests;
