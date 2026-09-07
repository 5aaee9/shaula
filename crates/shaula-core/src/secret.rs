//! Secret wrapper with redacted `Debug` and no response serialization.

use std::fmt;

use zeroize::{Zeroize, ZeroizeOnDrop};

/// A credential byte string that never reveals its content through `Debug`,
/// `Display` or serialization derives.
///
/// Cloning is explicit via [`SecretString::expose`] to keep copies minimal.
#[derive(Clone, Zeroize, ZeroizeOnDrop, PartialEq, Eq)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Grants access to the plaintext. Every call site is a secret handoff.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretString(REDACTED)")
    }
}

/// Bounded presence metadata; the only secret fact allowed on read surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialPresence {
    Present,
    Absent,
}

impl CredentialPresence {
    pub fn as_str(self) -> &'static str {
        match self {
            CredentialPresence::Present => "present",
            CredentialPresence::Absent => "absent",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_shows_value() {
        let secret = SecretString::new("github_pat_SUPERSECRET123");
        let rendered = format!("{secret:?}");
        assert_eq!(rendered, "SecretString(REDACTED)");
        assert!(!rendered.contains("SUPERSECRET"));
    }

    #[test]
    fn display_is_not_implemented_via_debug_string() {
        // Display is not implemented at all; ensure Debug path is used and
        // contains no plaintext.
        let secret = SecretString::new("-----BEGIN RSA PRIVATE KEY-----");
        let rendered = format!("{secret:?}");
        assert!(!rendered.contains("RSA"));
    }
}
