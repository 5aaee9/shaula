//! Store Forgejo runner identity separately from GitHub runner identity.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE forgejo_runner_identities (
                    generation_id VARCHAR(64) PRIMARY KEY NOT NULL,
                    runner_id BIGINT NOT NULL,
                    runner_uuid VARCHAR(128) NOT NULL,
                    created_at BIGINT NOT NULL,
                    updated_at BIGINT NOT NULL
                );",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("migrations are forward-only".into()))
    }
}
