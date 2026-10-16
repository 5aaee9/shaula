use crate::entities::lifecycle::{
    runner_generations as generations, setup_info_capabilities as capabilities,
};
use crate::query_ext::{same, JoinOn};
use crate::Store;
use async_trait::async_trait;
use sea_orm::{
    sea_query::{Expr, Query},
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QuerySelect,
};
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
        let creating = Query::select()
            .column(generations::Column::Id)
            .exprs([Expr::val(verifier), Expr::val(expires_at), Expr::val(now)])
            .from(generations::Entity)
            .and_where(generations::Column::Id.eq(generation_id))
            .and_where(generations::Column::State.is_in(["CreatePending", "Creating"]))
            .to_owned();
        let insert = Query::insert()
            .into_table(capabilities::Entity)
            .columns([
                capabilities::Column::GenerationId,
                capabilities::Column::Verifier,
                capabilities::Column::ExpiresAt,
                capabilities::Column::CreatedAt,
            ])
            .select_from(creating)
            .map_err(|_| unavailable())?
            .to_owned();
        let connection = self.store.connection();
        let result = connection
            .execute(connection.get_database_backend().build(&insert))
            .await
            .map_err(|_| unavailable())?;
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
        let expected = capabilities::Entity::find()
            .select_only()
            .column(capabilities::Column::Verifier)
            .inner_join_on(
                generations::Entity,
                same(generations::Column::Id, capabilities::Column::GenerationId),
            )
            .filter(capabilities::Column::GenerationId.eq(generation_id))
            .filter(capabilities::Column::ExpiresAt.gt(now))
            .filter(capabilities::Column::Revoked.eq(0))
            .filter(generations::Column::State.ne("Destroyed"))
            .into_tuple::<Vec<u8>>()
            .one(self.store.connection())
            .await
            .map_err(|_| unavailable())?;
        let Some(expected) = expected else {
            return Ok(false);
        };
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
