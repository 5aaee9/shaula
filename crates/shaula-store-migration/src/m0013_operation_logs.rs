//! Diagnostic records deliberately outlive resource and Workspace deletion.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(
            "CREATE TABLE operation_log_invocations (
              id TEXT PRIMARY KEY NOT NULL, generation_id TEXT NOT NULL,
              fleet_key TEXT NOT NULL, operation TEXT NOT NULL, ordinal INTEGER NOT NULL,
              record_json TEXT NOT NULL, sealed_at INTEGER, retained_bytes INTEGER NOT NULL DEFAULT 0,
              UNIQUE(generation_id,operation,ordinal)
             );
             CREATE INDEX operation_logs_generation ON operation_log_invocations(generation_id,ordinal);
             CREATE TABLE operation_log_chunks (
              invocation_id TEXT NOT NULL, sequence INTEGER NOT NULL, command_ordinal INTEGER NOT NULL,
              phase TEXT NOT NULL, stream TEXT NOT NULL, observed_at INTEGER NOT NULL,
              bytes INTEGER NOT NULL, digest TEXT NOT NULL, file_name TEXT NOT NULL,
              PRIMARY KEY(invocation_id,sequence)
             );
             CREATE INDEX operation_log_retention ON operation_log_invocations(sealed_at);"
        ).await?;
        Ok(())
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "operation history migrations are forward-only".into(),
        ))
    }
}
