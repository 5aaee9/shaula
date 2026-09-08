//! Retain exact execution authority independently of the latest handoff.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE fleet_auth_context_history (
                fleet_key TEXT NOT NULL, profile_key TEXT NOT NULL,
                revision BIGINT NOT NULL, context_json TEXT NOT NULL,
                PRIMARY KEY(fleet_key, profile_key, revision));
             INSERT INTO fleet_auth_context_history
                SELECT fleet_key, observed_profile_key, observed_revision, observed_context_json
                FROM fleet_auth_contexts WHERE observed_context_json IS NOT NULL
                AND observed_profile_key IS NOT NULL AND observed_revision IS NOT NULL;
             CREATE TABLE fleet_session_auth (
                fleet_key TEXT PRIMARY KEY NOT NULL, profile_key TEXT NOT NULL,
                revision BIGINT NOT NULL);
             INSERT INTO fleet_session_auth
                SELECT s.fleet_key, h.observed_profile_key, h.observed_revision
                FROM fleet_sessions s JOIN fleet_auth_handoffs h ON h.fleet_key=s.fleet_key
                WHERE h.observed_profile_key IS NOT NULL AND h.observed_revision IS NOT NULL;",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "execution context retention cannot be rolled back".into(),
        ))
    }
}
