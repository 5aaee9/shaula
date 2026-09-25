//! Credential provenance is supplied only by a trusted adapter, never by request JSON.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AuthenticationContext {
    #[default]
    System,
    OidcSession,
    OidcAccessToken,
    PersonalAccessToken(String),
}
impl AuthenticationContext {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::OidcSession => "oidc_session",
            Self::OidcAccessToken => "oidc_access_token",
            Self::PersonalAccessToken(_) => "personal_access_token",
        }
    }
    pub fn token_id(&self) -> Option<&str> {
        match self {
            Self::PersonalAccessToken(id) => Some(id),
            _ => None,
        }
    }
}

/// Authenticated principal and effective grants supplied by the HTTP adapter.
/// `name` is a versioned stable identity, not a mutable display name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Actor {
    pub authentication: AuthenticationContext,
    pub name: String,
    pub scopes: Vec<Scope>,
}

impl Actor {
    pub fn has(&self, scope: Scope) -> bool {
        self.scopes.contains(&scope)
    }
}

/// Independent management capabilities (spec 0005 §8). High-trust
/// capabilities are separately grantable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    FleetRead,
    LogsRead,
    FleetWrite,
    FleetRetire,
    TemplateRead,
    TemplatePublish,
    TemplateAttest,
    TemplateRetire,
    AuthRead,
    AuthWrite,
    AuthRetire,
    AccessTokenRead,
    AccessTokenWrite,
    AccessTokenRevoke,
}

impl Scope {
    pub const ALL: [Self; 14] = [
        Self::FleetRead,
        Self::LogsRead,
        Self::FleetWrite,
        Self::FleetRetire,
        Self::TemplateRead,
        Self::TemplatePublish,
        Self::TemplateAttest,
        Self::TemplateRetire,
        Self::AuthRead,
        Self::AuthWrite,
        Self::AuthRetire,
        Self::AccessTokenRead,
        Self::AccessTokenWrite,
        Self::AccessTokenRevoke,
    ];
    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == raw)
    }
    pub fn delegable(self) -> bool {
        self != Self::AccessTokenWrite
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Scope::FleetRead => "fleet.read",
            Scope::LogsRead => "logs.read",
            Scope::FleetWrite => "fleet.write",
            Scope::FleetRetire => "fleet.retire",
            Scope::TemplateRead => "template.read",
            Scope::TemplatePublish => "template.publish",
            Scope::TemplateAttest => "template.attest",
            Scope::TemplateRetire => "template.retire",
            Scope::AuthRead => "auth.read",
            Scope::AuthWrite => "auth.write",
            Scope::AuthRetire => "auth.retire",
            Scope::AccessTokenRead => "access-token.read",
            Scope::AccessTokenWrite => "access-token.write",
            Scope::AccessTokenRevoke => "access-token.revoke",
        }
    }
}
