//! GitLab Pool state (spec 0043): runner identities, the per-revision demand
//! anchor, and the Verified job association recorded before pausing.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE gitlab_runner_identities (
                    generation_id VARCHAR(64) PRIMARY KEY NOT NULL,
                    runner_id BIGINT NOT NULL,
                    created_at BIGINT NOT NULL,
                    updated_at BIGINT NOT NULL
                );
                 CREATE TABLE gitlab_anchors (
                    fleet_key VARCHAR(128) NOT NULL,
                    fleet_revision BIGINT NOT NULL,
                    anchor_name VARCHAR(255) NOT NULL,
                    runner_id BIGINT,
                    state VARCHAR(32) NOT NULL,
                    created_at BIGINT NOT NULL,
                    updated_at BIGINT NOT NULL,
                    PRIMARY KEY (fleet_key, fleet_revision)
                );
                 CREATE TABLE gitlab_job_associations (
                    generation_id VARCHAR(64) PRIMARY KEY NOT NULL,
                    job_id BIGINT NOT NULL,
                    project_id BIGINT NOT NULL,
                    pipeline_id BIGINT NOT NULL,
                    observed_at BIGINT NOT NULL
                );",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("migrations are forward-only".into()))
    }
}
