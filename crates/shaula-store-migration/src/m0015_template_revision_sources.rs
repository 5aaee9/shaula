//! Recover only unambiguous content associations from the pre-upgrade catalog.
//! The source key is historical metadata, without a foreign key to that catalog.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE template_profile_revisions ADD COLUMN source_key TEXT;
                 UPDATE template_profile_revisions
                 SET source_key = (
                     SELECT MIN(source.key) FROM template_sources AS source
                     WHERE source.artifact_digest = template_profile_revisions.artifact_digest
                       AND source.engine_ref = template_profile_revisions.engine_ref
                       AND source.platform = template_profile_revisions.platform
                 )
                 WHERE platform IS NOT NULL AND platform <> ''
                   AND (
                     SELECT COUNT(*) FROM template_sources AS source
                     WHERE source.artifact_digest = template_profile_revisions.artifact_digest
                       AND source.engine_ref = template_profile_revisions.engine_ref
                       AND source.platform = template_profile_revisions.platform
                   ) = 1;",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("migrations are forward-only".into()))
    }
}
