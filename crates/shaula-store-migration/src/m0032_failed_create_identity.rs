//! Failed-Create destroy anchor (spec 0010 §7): the state identity of a
//! Create that failed after apply but whose bound state provably records the
//! template's whole managed resource shape, so Destroy can settle it.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE runner_generations ADD COLUMN failed_create_identity_json TEXT",
            )
            .await?;
        Ok(())
    }
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("migrations are forward-only".into()))
    }
}
