use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(
            "ALTER TABLE audit_records ADD COLUMN credential_kind TEXT NOT NULL DEFAULT 'unknown';
             ALTER TABLE audit_records ADD COLUMN access_token_id TEXT;"
        ).await?;
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("migrations are forward-only".into()))
    }
}
