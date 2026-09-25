//! Preserve unowned legacy records as tombstones; never guess their principal.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE idempotency_records ADD COLUMN principal TEXT;
             ALTER TABLE idempotency_records ADD COLUMN operation TEXT NOT NULL DEFAULT 'legacy';
             DROP INDEX idx_idem_unique;
             CREATE UNIQUE INDEX idx_idem_principal ON idempotency_records
                 (principal, operation, resource_kind, resource_key, idempotency_key);
             CREATE INDEX idx_idem_legacy ON idempotency_records
                 (resource_kind, resource_key, idempotency_key) WHERE principal IS NULL;",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("migrations are forward-only".into()))
    }
}
