use crate::Store;
use async_trait::async_trait;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use sha2::{Digest, Sha256};
use shaula_core::{
    error::{CoreError, CoreResult, ReasonCode},
    setup_info::{SetupInfoAuthorizer, SetupInfoConfig, SetupInfoIssuer},
    template::SetupInfoDescriptor,
};
use subtle::ConstantTimeEq;

#[derive(Clone)]
pub struct SetupInfoCapabilityRegistry {
    store: Store,
    config: SetupInfoConfig,
}

impl SetupInfoCapabilityRegistry {
    pub fn new(store: Store, config: SetupInfoConfig) -> CoreResult<Self> {
        config.validate().map_err(|_| unavailable())?;
        Ok(Self { store, config })
    }
}

#[async_trait]
impl SetupInfoIssuer for SetupInfoCapabilityRegistry {
    async fn issue(&self, generation_id: &str, now: i64) -> CoreResult<SetupInfoDescriptor> {
        // Refuse millisecond epochs instead of accidentally issuing a token for millennia.
        if !(1..=253_402_300_799).contains(&now) {
            return Err(unavailable());
        }
        uuid::Uuid::parse_str(generation_id).map_err(|_| unavailable())?;
        let capability = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let verifier = Sha256::digest(capability.as_bytes()).to_vec();
        let expires_at = now
            .checked_add(i64::from(self.config.capability_ttl_seconds))
            .ok_or_else(unavailable)?;
        let url = format!(
            "{}/runner/v1/generations/{generation_id}/setup-info",
            self.config.advertised_origin.trim_end_matches('/')
        );
        let descriptor =
            SetupInfoDescriptor::enabled(url, capability, expires_at, self.config.wait_seconds)?;
        // A duplicate issue must not rotate the token frozen in original recovery inputs.
        let result = self.store.connection().execute(Statement::from_sql_and_values(DbBackend::Sqlite,
            "INSERT INTO setup_info_capabilities(generation_id,verifier,expires_at,created_at)
             SELECT id,?,?,? FROM runner_generations WHERE id=? AND state IN ('CreatePending','Creating')",
            [verifier.into(), expires_at.into(), now.into(), generation_id.into()])).await.map_err(|_| unavailable())?;
        if result.rows_affected() != 1 {
            return Err(unavailable());
        }
        Ok(descriptor)
    }
}

#[async_trait]
impl SetupInfoAuthorizer for SetupInfoCapabilityRegistry {
    async fn authorize(&self, generation_id: &str, capability: &str, now: i64) -> CoreResult<bool> {
        if uuid::Uuid::parse_str(generation_id).is_err()
            || capability.len() != 64
            || !capability.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Ok(false);
        }
        let row = self.store.connection().query_one(Statement::from_sql_and_values(DbBackend::Sqlite,
            "SELECT c.verifier FROM setup_info_capabilities c JOIN runner_generations g ON g.id=c.generation_id
             WHERE c.generation_id=? AND c.expires_at>? AND c.revoked=0 AND g.state!='Destroyed'",
            [generation_id.into(), now.into()])).await.map_err(|_| unavailable())?;
        let Some(row) = row else {
            return Ok(false);
        };
        let expected = row
            .try_get::<Vec<u8>>("", "verifier")
            .map_err(|_| unavailable())?;
        let actual = Sha256::digest(capability.as_bytes());
        Ok(expected.as_slice().ct_eq(actual.as_slice()).into())
    }
}

fn unavailable() -> CoreError {
    CoreError::new(ReasonCode::Internal, "setup-info capability unavailable")
}

#[cfg(test)]
#[path = "setup_info_tests.rs"]
mod tests;
