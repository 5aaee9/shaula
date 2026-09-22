//! Forgejo job snapshots never share GitHub message/assignment identities.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(
            "CREATE TABLE forgejo_job_polls (
                scope_key TEXT PRIMARY KEY NOT NULL, observed_at INTEGER NOT NULL,
                attempted_at INTEGER NOT NULL, failed INTEGER NOT NULL
             );
             CREATE TABLE forgejo_workflow_jobs (
                id TEXT PRIMARY KEY NOT NULL, scope_key TEXT NOT NULL,
                fleet_key TEXT NOT NULL, fleet_incarnation TEXT NOT NULL,
                protocol_job_id TEXT NOT NULL, summary_json TEXT NOT NULL,
                status TEXT NOT NULL, repository TEXT, job_name TEXT,
                in_snapshot INTEGER NOT NULL,
                created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
                UNIQUE(scope_key,protocol_job_id)
             );
             CREATE INDEX idx_forgejo_jobs_page ON forgejo_workflow_jobs(created_at DESC,id DESC);
             CREATE INDEX idx_forgejo_jobs_fleet ON forgejo_workflow_jobs(fleet_key,status,created_at DESC,id DESC);
             CREATE INDEX idx_forgejo_jobs_present ON forgejo_workflow_jobs(scope_key,in_snapshot);
             CREATE TABLE forgejo_job_observations (
                id TEXT PRIMARY KEY NOT NULL, job_record_id TEXT NOT NULL,
                data_json TEXT NOT NULL, observed_at INTEGER NOT NULL
             );
             CREATE INDEX idx_forgejo_observation_job ON forgejo_job_observations(job_record_id,observed_at,id);"
        ).await?;
        Ok(())
    }
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("migrations are forward-only".into()))
    }
}
