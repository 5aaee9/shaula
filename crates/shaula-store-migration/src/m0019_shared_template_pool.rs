use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        // Shared TemplatePool resource heads (spec 0037 §2): a pool owns no
        // runners, so its lifecycle carries no mutation fence — revisions
        // are append-only and deletion is a tombstone after the reference
        // check.
        db.execute_unprepared(
            "CREATE TABLE template_pools (
            key VARCHAR(128) PRIMARY KEY,
            incarnation VARCHAR(64) NOT NULL,
            desired_revision BIGINT NOT NULL,
            observed_revision BIGINT NOT NULL DEFAULT 0,
            phase VARCHAR(32) NOT NULL DEFAULT 'Pending',
            deletion_marker BOOLEAN NOT NULL DEFAULT 0,
            tombstone BOOLEAN NOT NULL DEFAULT 0,
            created_at BIGINT NOT NULL,
            updated_at BIGINT NOT NULL
        )",
        )
        .await?;
        db.execute_unprepared(
            "CREATE TABLE template_pool_revisions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            pool_key VARCHAR(128) NOT NULL,
            revision BIGINT NOT NULL,
            spec_json TEXT NOT NULL,
            failure_policy VARCHAR(32) NOT NULL,
            actor VARCHAR(128),
            created_at BIGINT NOT NULL,
            UNIQUE(pool_key, revision)
        )",
        )
        .await?;
        // Immutable member rows mirroring fleet_revision_pool_members, keyed
        // by the pool revision instead of the fleet revision (spec 0037 §3).
        db.execute_unprepared(
            "CREATE TABLE template_pool_members (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            pool_key VARCHAR(128) NOT NULL,
            pool_revision BIGINT NOT NULL,
            member_key VARCHAR(128) NOT NULL,
            template_profile_key VARCHAR(128) NOT NULL,
            template_revision BIGINT NOT NULL,
            template_artifact_digest VARCHAR(256) NOT NULL,
            template_attestation_id VARCHAR(256) NOT NULL,
            template_inputs_json TEXT NOT NULL,
            inputs_digest VARCHAR(256) NOT NULL,
            weight BIGINT NOT NULL,
            max_runners BIGINT,
            UNIQUE(pool_key, pool_revision, member_key)
        )",
        )
        .await?;
        db.execute_unprepared(
            "ALTER TABLE fleet_revisions ADD COLUMN template_pool_ref VARCHAR(128)",
        )
        .await?;
        db.execute_unprepared(
            "ALTER TABLE fleet_revisions ADD COLUMN template_pool_revision BIGINT",
        )
        .await?;
        Ok(())
    }
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("migrations are forward-only".into()))
    }
}
