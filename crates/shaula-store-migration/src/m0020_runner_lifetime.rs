//! Persistent hard-lifetime and two-phase forced cleanup checkpoints.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE runner_generations ADD COLUMN provisioned_at BIGINT;
                 ALTER TABLE runner_generations ADD COLUMN expiry_requested_at BIGINT;
                 ALTER TABLE runner_generations ADD COLUMN resources_destroyed_at BIGINT;
                 UPDATE runner_generations
                 SET provisioned_at = COALESCE(
                     (SELECT MIN(updated_at) FROM runner_operations
                      WHERE generation_id = runner_generations.id AND kind = 'Create'
                        AND state IN ('Succeeded', 'Completed')),
                     updated_at)
                 WHERE shaula_result_json IS NOT NULL;",
            )
            .await?;
        // Legacy rows lack an exact success timestamp. The earliest completed
        // Create (or last persisted observation) is a conservative, one-time
        // anchor; no restart may grant another grace period.
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("migrations are forward-only".into()))
    }
}
