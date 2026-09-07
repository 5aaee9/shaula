use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;

use super::{StateError, StateResult};
use crate::secret::SecretString;

/// Backend ownership, not a process-death proof or a Terraform lock ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateClaim {
    pub generation_id: Uuid,
    pub worker_epoch: i64,
    pub worker_attempt: Uuid,
}

/// A state-only capability. The prefix is protocol separation, not authority;
/// only the verifier attached to the current Claim authenticates it.
#[derive(Debug, Clone)]
pub struct StateCapability(SecretString);

impl StateCapability {
    /// Two independently random UUIDs provide 244 random bits. Plaintext is
    /// handed to the worker once; only the verifier belongs in the database.
    pub fn issue() -> Self {
        Self(SecretString::new(format!(
            "ss1_{}{}",
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        )))
    }

    pub fn parse(secret: SecretString) -> StateResult<Self> {
        let Some(random) = secret.expose().strip_prefix("ss1_") else {
            return Err(StateError::Unauthorized);
        };
        if random.len() != 64 || !random.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(StateError::Unauthorized);
        }
        Ok(Self(secret))
    }

    /// Narrow credential handoff (Basic password / TF_HTTP_PASSWORD only).
    pub fn expose(&self) -> &str {
        self.0.expose()
    }

    pub fn verifier(&self) -> Vec<u8> {
        let mut hash = Sha256::new();
        hash.update(b"shaula-state-capability-v1\0");
        hash.update(self.expose().as_bytes());
        hash.finalize().to_vec()
    }

    pub fn matches(&self, verifier: &[u8]) -> bool {
        bool::from(self.verifier().ct_eq(verifier))
    }
}

#[derive(Debug, Clone)]
pub struct StateAccess {
    pub generation_id: Uuid,
    pub capability: StateCapability,
}
