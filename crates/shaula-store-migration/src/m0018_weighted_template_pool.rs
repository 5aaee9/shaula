use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "ALTER TABLE runner_generations ADD COLUMN pool_member_key VARCHAR(128)",
        )
        .await?;
        db.execute_unprepared(
            "CREATE TABLE fleet_revision_pool_members (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            fleet_key VARCHAR(128) NOT NULL,
            fleet_revision BIGINT NOT NULL,
            member_key VARCHAR(128) NOT NULL,
            template_profile_key VARCHAR(128) NOT NULL,
            template_revision BIGINT NOT NULL,
            template_artifact_digest VARCHAR(256) NOT NULL,
            template_attestation_id VARCHAR(256) NOT NULL,
            template_inputs_json TEXT NOT NULL,
            inputs_digest VARCHAR(256) NOT NULL,
            weight BIGINT NOT NULL,
            max_runners BIGINT,
            UNIQUE(fleet_key, fleet_revision, member_key)
        )",
        )
        .await?;
        Ok(())
    }
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("migrations are forward-only".into()))
    }
}
