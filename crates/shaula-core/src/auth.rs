//! GitHub Auth Profile identities: kind, immutable revision refs and target
//! allowlists.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{CoreError, CoreResult, ReasonCode};
use crate::github::GitHubTarget;
use crate::secret::SecretString;

/// Stable logical key of a GitHub Auth Profile.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AuthProfileKey(String);

impl AuthProfileKey {
    /// Strict parse (R8-01): the value must ALREADY be a canonical
    /// stable identifier — the constructor never rewrites it (no trim,
    /// no case folding). A key that whitespace normalization would have
    /// changed is REJECTED at the boundary instead of being silently
    /// persisted under a different identity than the one validated.
    pub fn new(value: impl Into<String>) -> CoreResult<Self> {
        let value = value.into();
        if !is_stable_identifier(&value) {
            return Err(CoreError::new(
                ReasonCode::SpecInvalid,
                "auth profile key is not a stable identifier",
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Auth credential kind. Fixed per profile incarnation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    GithubApp,
    Pat,
}

impl AuthKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AuthKind::GithubApp => "github_app",
            AuthKind::Pat => "pat",
        }
    }
}

/// Immutable, indivisible `(profile_key, revision)` reference. Desired and
/// observed auth state are always compared, persisted and released as full
/// tuples — never by bare revision numbers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AuthRevisionRef {
    pub profile_key: AuthProfileKey,
    pub revision: u64,
}

impl AuthRevisionRef {
    pub fn new(profile_key: AuthProfileKey, revision: u64) -> Self {
        Self {
            profile_key,
            revision,
        }
    }
}

impl std::fmt::Display for AuthRevisionRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/rev{}", self.profile_key.as_str(), self.revision)
    }
}

/// Normalized target allowlist fixed within one Auth Profile incarnation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetAllowlist {
    pub targets: Vec<GitHubTarget>,
}

impl TargetAllowlist {
    pub fn new(targets: Vec<GitHubTarget>) -> CoreResult<Self> {
        if targets.is_empty() {
            return Err(CoreError::new(
                ReasonCode::SpecInvalid,
                "target allowlist must not be empty",
            ));
        }
        Ok(Self { targets })
    }

    /// Whether the allowlist covers the target. Exact match semantics:
    /// a repository target is not implied by its organization entry.
    pub fn allows(&self, target: &GitHubTarget) -> bool {
        self.targets.iter().any(|t| t == target)
    }
}

/// Non-secret App identity fixed per incarnation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppIdentity {
    pub app_id: String,
    pub installation_id: i64,
}

/// Non-secret PAT principal identity; the PAT itself never lives here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatIdentity {
    pub principal: String,
}

/// The immutable credential revision body stored in SQLite. Secret bytes are
/// write-only: never serialized into responses, audit or telemetry.
#[derive(Clone)]
pub struct AuthCredentialRevision {
    pub profile_key: AuthProfileKey,
    pub revision: u64,
    pub kind: AuthKind,
    pub app_identity: Option<AppIdentity>,
    pub pat_identity: Option<PatIdentity>,
    pub allowlist: TargetAllowlist,
    pub secret: SecretString,
}

impl std::fmt::Debug for AuthCredentialRevision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthCredentialRevision")
            .field("profile_key", &self.profile_key)
            .field("revision", &self.revision)
            .field("kind", &self.kind)
            .field("credential_present", &true)
            .finish_non_exhaustive()
    }
}

impl AuthCredentialRevision {
    pub fn revision_ref(&self) -> AuthRevisionRef {
        AuthRevisionRef::new(self.profile_key.clone(), self.revision)
    }
}

/// Classification outcome of asynchronous credential validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialValidationOutcome {
    Accepted,
    CredentialMalformed,
    Unauthenticated,
    PermissionDenied,
    TargetHiddenOrNotFound,
    RateLimited,
}

impl CredentialValidationOutcome {
    pub fn reason(self) -> crate::error::ReasonCode {
        use crate::error::ReasonCode as R;
        match self {
            CredentialValidationOutcome::Accepted => R::SpecAccepted,
            CredentialValidationOutcome::CredentialMalformed => R::CredentialMalformed,
            CredentialValidationOutcome::Unauthenticated => R::Unauthenticated,
            CredentialValidationOutcome::PermissionDenied => R::PermissionDenied,
            CredentialValidationOutcome::TargetHiddenOrNotFound => R::TargetHiddenOrNotFound,
            CredentialValidationOutcome::RateLimited => R::RateLimited,
        }
    }
}

/// Opaque generation identity for attempts and attestations.
pub fn new_attempt_id() -> String {
    Uuid::new_v4().to_string()
}

/// Canonical request/record hash over ordered byte parts: sha256 over a
/// LENGTH-PREFIXED encoding (`<decimal-len>:<bytes>` per part),
/// `sha256:`-prefixed. The length prefix makes the encoding INJECTIVE
/// over arbitrary bytes (R7-05): no two distinct part tuples — not even
/// ones whose bytes differ only around an embedded separator such as a
/// NUL in a URI path segment — can ever produce the same digest. THE
/// single hashing authority for canonical identity material (request
/// idempotency hashes, attestation record ids) — every caller must use
/// this so hashes stay comparable.
pub fn request_hash_parts(parts: &[&[u8]]) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    for part in parts {
        hasher.update(part.len().to_string().as_bytes());
        hasher.update(b":");
        hasher.update(part);
        hasher.update(b"|");
    }
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

/// THE stable-identifier charset shared by every request-key validator
/// (S1): non-empty, ≤128 bytes, ASCII alphanumerics plus `-` `_` `.` —
/// the characters a canonical URI path identity segment may contain.
/// One predicate owner, so profile, fleet and attestation keys can never
/// drift into different identity rules.
pub(crate) fn is_stable_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

pub fn validate_profile_key_for_fleet_ref(key: &str) -> CoreResult<()> {
    AuthProfileKey::new(key).map(|_| ())
}

/// Validates an attestation resource key from the request URI (R7-05):
/// the same stable-identifier charset as a profile key, so bytes that
/// are not legitimate URI path segments (control bytes such as NUL) are
/// rejected at the request boundary instead of entering durable
/// identity material. Distinct validator, same predicate (C28: the
/// domain reasons stay separate).
pub fn validate_attestation_key(key: &str) -> CoreResult<()> {
    if !is_stable_identifier(key) {
        return Err(CoreError::new(
            ReasonCode::SpecInvalid,
            "attestation key is not a stable identifier",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn request_hash_parts_encoding_is_injective_over_separator_bytes() {
        // R7-05: two DIFFERENT part tuples whose concatenated bytes used
        // to coincide around a NUL separator must never share a digest.
        let a = request_hash_parts(&[b"a", b"1", b"2\x00x"]);
        let b = request_hash_parts(&[b"a\x001", b"2", b"x"]);
        assert_ne!(a, b);
        // And the encoding still hashes the same tuple stably.
        assert_eq!(
            request_hash_parts(&[b"x", b"y"]),
            request_hash_parts(&[b"x", b"y"])
        );
        // An embedded NUL inside one part changes the digest.
        assert_ne!(request_hash_parts(&[b"x"]), request_hash_parts(&[b"x\0"]));
    }

    #[test]
    fn attestation_key_rejects_control_and_foreign_bytes() {
        assert!(validate_attestation_key("att-2").is_ok());
        assert!(validate_attestation_key("runners.east_01").is_ok());
        assert!(validate_attestation_key("2\x00x").is_err());
        assert!(validate_attestation_key("").is_err());
        assert!(validate_attestation_key("sp ace").is_err());
        assert!(validate_attestation_key("k/slash").is_err());
    }

    #[test]
    fn auth_ref_tuple_round_trip() {
        let r = AuthRevisionRef::new(AuthProfileKey::new("prod-app").unwrap(), 7);
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"profile_key":"prod-app","revision":7}"#
        );
    }

    #[test]
    fn bare_revision_never_equals_full_tuple() {
        let a = AuthRevisionRef::new(AuthProfileKey::new("p1").unwrap(), 3);
        let b = AuthRevisionRef::new(AuthProfileKey::new("p2").unwrap(), 3);
        assert_ne!(
            a, b,
            "same revision number in a different profile is a different ref"
        );
    }

    #[test]
    fn allowlist_exact_target_matching() {
        let allowlist = TargetAllowlist::new(vec![
            GitHubTarget::organization("example-org").unwrap(),
            GitHubTarget::new_repository("other-org", "repo").unwrap(),
        ])
        .unwrap();
        assert!(allowlist.allows(&GitHubTarget::organization("example-org").unwrap()));
        assert!(allowlist.allows(&GitHubTarget::new_repository("other-org", "repo").unwrap()));
        // A repository is not implied by its organization entry.
        assert!(!allowlist.allows(&GitHubTarget::new_repository("example-org", "child").unwrap()));
        assert!(!allowlist.allows(&GitHubTarget::organization("other-org").unwrap()));
    }

    #[test]
    fn secret_never_leaks_in_revision_debug() {
        let revision = AuthCredentialRevision {
            profile_key: AuthProfileKey::new("k").unwrap(),
            revision: 1,
            kind: AuthKind::Pat,
            app_identity: None,
            pat_identity: Some(PatIdentity {
                principal: "octocat".into(),
            }),
            allowlist: TargetAllowlist::new(vec![GitHubTarget::organization("o").unwrap()])
                .unwrap(),
            secret: SecretString::new("ghp_supersecret"),
        };
        let rendered = format!("{revision:?}");
        assert!(!rendered.contains("ghp_supersecret"));
    }

    #[test]
    fn invalid_profile_keys_rejected() {
        assert!(AuthProfileKey::new("").is_err());
        assert!(AuthProfileKey::new("has space").is_err());
        assert!(AuthProfileKey::new("ok-key.1_2").is_ok());
        // R8-01: the constructor NEVER rewrites the value — a key that
        // trimming would change is rejected, so the validated bytes are
        // always exactly the persisted bytes.
        assert!(AuthProfileKey::new("\na").is_err());
        assert!(AuthProfileKey::new("a\n").is_err());
        assert!(AuthProfileKey::new(" a").is_err());
        assert!(AuthProfileKey::new("\ta\t").is_err());
        assert_eq!(AuthProfileKey::new("a").unwrap().as_str(), "a");
    }
}

/// Parameter object for creating an Auth Candidate revision.
#[derive(Debug, Clone)]
pub struct AuthRevisionInsert {
    pub key: String,
    pub incarnation: String,
    pub revision: i64,
    pub kind: String,
    pub app_id: Option<String>,
    pub installation_id: Option<i64>,
    pub pat_principal: Option<String>,
    pub allowlist_json: String,
}
