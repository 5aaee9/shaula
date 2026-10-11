//! Additive lookup indexes: open operations per generation and fleet
//! revisions per shared-pool revision. No data or constraint changes.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_runner_operations_generation
                    ON runner_operations(generation_id, state);
                 CREATE INDEX IF NOT EXISTS idx_fleet_revisions_pool_ref
                    ON fleet_revisions(template_pool_ref, template_pool_revision);",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("migrations are forward-only".into()))
    }
}
