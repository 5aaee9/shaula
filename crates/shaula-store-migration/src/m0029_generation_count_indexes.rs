//! Additive lookup indexes for the Generation counting aggregates: fleet
//! occupancy/effective counts and per-member usage, plus the shared-pool
//! join from fleet revisions to their generations. No data or constraint
//! changes.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_runner_generations_fleet_state
                    ON runner_generations(fleet_key, state, pool_member_key);
                 CREATE INDEX IF NOT EXISTS idx_runner_generations_fleet_revision
                    ON runner_generations(fleet_key, fleet_revision);",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("migrations are forward-only".into()))
    }
}
