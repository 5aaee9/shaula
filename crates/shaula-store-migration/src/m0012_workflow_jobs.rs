//! Retained observations are independent of sessions and runner workspaces.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(
            "ALTER TABLE listener_messages ADD COLUMN payload_digest_version INTEGER NOT NULL DEFAULT 1
                CHECK(payload_digest_version IN (1,2));
             CREATE TABLE workflow_jobs (
                id TEXT PRIMARY KEY NOT NULL, scope_key TEXT NOT NULL,
                fleet_key TEXT NOT NULL, fleet_incarnation TEXT NOT NULL,
                scale_set_id INTEGER NOT NULL, protocol_job_id TEXT NOT NULL,
                summary_json TEXT NOT NULL, status TEXT NOT NULL,
                repository TEXT, job_name TEXT,
                created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
                UNIQUE(scope_key,protocol_job_id)
             );
             CREATE INDEX idx_workflow_jobs_page ON workflow_jobs(created_at DESC,id DESC);
             CREATE INDEX idx_workflow_jobs_fleet ON workflow_jobs(fleet_key,status,created_at DESC,id DESC);
             CREATE TABLE workflow_job_observations (
                id TEXT PRIMARY KEY NOT NULL, scope_key TEXT NOT NULL,
                job_record_id TEXT, protocol_job_id TEXT NOT NULL,
                runner_request_id INTEGER NOT NULL, runner_id INTEGER,
                data_json TEXT NOT NULL, observed_at INTEGER NOT NULL
             );
             CREATE INDEX idx_workflow_observation_job ON workflow_job_observations(job_record_id,observed_at,id);
             CREATE INDEX idx_workflow_observation_request ON workflow_job_observations(scope_key,runner_request_id);
             CREATE INDEX idx_workflow_observation_runner ON workflow_job_observations(scope_key,runner_id);
             CREATE INDEX idx_workflow_observation_generation ON workflow_job_observations(
                json_extract(data_json,'$.generation_id'),json_extract(data_json,'$.association_status'));
             CREATE TABLE workflow_generation_identity (
                generation_id TEXT PRIMARY KEY NOT NULL, scope_key TEXT NOT NULL,
                fleet_incarnation TEXT NOT NULL, github_runner_id INTEGER
             );
             CREATE INDEX idx_workflow_generation_runner ON workflow_generation_identity(scope_key,github_runner_id);
             CREATE INDEX idx_workflow_generation_page ON runner_generations(created_at DESC,id DESC);"
        ).await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("migrations are forward-only".into()))
    }
}
