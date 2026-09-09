//! Separate a previously proven owned ID from an observed candidate ID.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE scale_set_state ADD COLUMN owned_scale_set_id INTEGER;
                 UPDATE scale_set_state SET owned_scale_set_id = scale_set_id
                 WHERE state = 'Adopted' AND scale_set_id > 0;",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("migrations are forward-only".into()))
    }
}
