//! Verifier-only, generation-bound setup-log access. No original bearer is stored here.
use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE setup_info_capabilities (
                generation_id TEXT PRIMARY KEY NOT NULL REFERENCES runner_generations(id),
                verifier BLOB NOT NULL CHECK(length(verifier)=32),
                expires_at INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                revoked INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX idx_setup_info_expiry ON setup_info_capabilities(expires_at);",
            )
            .await?;
        Ok(())
    }
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("migrations are forward-only".into()))
    }
}
