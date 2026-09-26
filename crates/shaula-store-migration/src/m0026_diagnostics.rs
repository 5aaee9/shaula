//! Disposable latest observations; no lifecycle rows or credentials are changed.
use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE diagnostic_snapshots (
                subject_key TEXT NOT NULL, lane TEXT NOT NULL, question TEXT NOT NULL,
                epoch TEXT NOT NULL, sequence INTEGER NOT NULL, observed_at INTEGER NOT NULL,
                payload TEXT NOT NULL CHECK(length(payload) <= 65536),
                PRIMARY KEY(subject_key, lane, question));
             CREATE INDEX diagnostic_retention ON diagnostic_snapshots(observed_at);
             CREATE INDEX diagnostic_generation ON diagnostic_snapshots(json_extract(payload,'$.guard.generation_id'), observed_at);",
            )
            .await?;
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("migrations are forward-only".into()))
    }
}
