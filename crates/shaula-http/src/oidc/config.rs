use shaula_core::{registry::Scope, secret::SecretString};
use url::Url;

use super::AuthError;

#[derive(Clone)]
pub struct Grant {
    pub issuer: String,
    pub subject: String,
    pub scopes: Vec<String>,
}

pub struct OidcConfig {
    pub(super) issuer: String,
    pub(super) client_id: String,
    pub(super) secret: SecretString,
    pub(super) origin: String,
    pub(super) audience: String,
    pub(super) grants: Vec<Grant>,
}

impl OidcConfig {
    pub fn new(
        issuer: String,
        client_id: String,
        secret: SecretString,
        origin: String,
        audience: String,
        grants: Vec<Grant>,
    ) -> Result<Self, AuthError> {
        let provider = https_url(&issuer).map_err(|_| AuthError::Configuration("provider"))?;
        if provider.query().is_some() {
            return Err(AuthError::Configuration("provider"));
        }
        let public = https_url(&origin).map_err(|_| AuthError::Configuration("public URL"))?;
        if public.path() != "/" || public.query().is_some() {
            return Err(AuthError::Configuration("public URL"));
        }
        for (name, value) in [
            ("client ID", client_id.as_str()),
            ("client secret", secret.expose()),
            ("API audience", audience.as_str()),
        ] {
            if value.trim().is_empty() || value.len() > 4096 {
                return Err(AuthError::Configuration(name));
            }
        }
        if client_id == audience {
            return Err(AuthError::Configuration(
                "API audience must differ from client ID",
            ));
        }
        if grants.len() > 1024 {
            return Err(AuthError::Configuration("authorization grants"));
        }
        let mut principals = std::collections::HashSet::new();
        for grant in &grants {
            if grant.issuer != issuer
                || grant.subject.is_empty()
                || grant.subject.len() > 256
                || !principals.insert(grant.subject.clone())
                || grant.scopes.len() > 11
                || grant.scopes.iter().any(|s| parse_scope(s).is_none())
            {
                return Err(AuthError::Configuration("authorization grants"));
            }
        }
        Ok(Self {
            issuer,
            client_id,
            secret,
            origin: public.origin().ascii_serialization(),
            audience,
            grants,
        })
    }

    pub(super) fn scopes(&self, subject: &str) -> Vec<Scope> {
        self.grants
            .iter()
            .find(|g| g.subject == subject)
            .map(|g| g.scopes.iter().filter_map(|s| parse_scope(s)).collect())
            .unwrap_or_default()
    }
}

pub(super) fn https_url(raw: &str) -> Result<Url, AuthError> {
    let url = Url::parse(raw).map_err(|_| AuthError::Configuration("HTTPS URL"))?;
    if raw.len() > 2048
        || raw.trim() != raw
        || raw.chars().any(char::is_whitespace)
        || raw.contains('\\')
        || url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(AuthError::Configuration("HTTPS URL"));
    }
    Ok(url)
}

fn parse_scope(raw: &str) -> Option<Scope> {
    [
        Scope::FleetRead,
        Scope::LogsRead,
        Scope::FleetWrite,
        Scope::FleetRetire,
        Scope::TemplateRead,
        Scope::TemplatePublish,
        Scope::TemplateAttest,
        Scope::TemplateRetire,
        Scope::AuthRead,
        Scope::AuthWrite,
        Scope::AuthRetire,
    ]
    .into_iter()
    .find(|s| s.as_str() == raw)
}
