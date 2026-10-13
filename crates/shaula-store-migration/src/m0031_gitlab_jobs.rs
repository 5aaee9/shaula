//! GitLab Jobs projection (spec 0043 §6.1): records scoped by Fleet
//! incarnation, instance, scope and Auth Profile, plus append-only Verified
//! Generation links that a later `runner: null` read can never clear.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(
            "CREATE TABLE gitlab_job_polls (
                scope_key TEXT PRIMARY KEY NOT NULL,
                enrichment_attempted_at INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE gitlab_workflow_jobs (
                id TEXT PRIMARY KEY NOT NULL, scope_key TEXT NOT NULL,
                fleet_key TEXT NOT NULL, fleet_incarnation TEXT NOT NULL,
                protocol_job_id TEXT NOT NULL, summary_json TEXT NOT NULL,
                status TEXT NOT NULL, repository TEXT, job_name TEXT,
                created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
                UNIQUE(scope_key,protocol_job_id)
             );
             CREATE INDEX idx_gitlab_jobs_page ON gitlab_workflow_jobs(created_at DESC,id DESC);
             CREATE INDEX idx_gitlab_jobs_fleet ON gitlab_workflow_jobs(fleet_key,status,created_at DESC,id DESC);
             CREATE INDEX idx_gitlab_jobs_pending ON gitlab_workflow_jobs(scope_key,status);
             CREATE TABLE gitlab_job_generations (
                job_record_id TEXT NOT NULL, generation_id TEXT NOT NULL,
                runner_id INTEGER NOT NULL, observed_at INTEGER NOT NULL,
                PRIMARY KEY(job_record_id,generation_id)
             );
             CREATE INDEX idx_gitlab_job_generation ON gitlab_job_generations(generation_id);
             CREATE TRIGGER gitlab_job_generations_immutable BEFORE UPDATE ON gitlab_job_generations
             BEGIN SELECT RAISE(ABORT,'GitLab job association is immutable'); END;
             CREATE TRIGGER gitlab_job_generations_retained BEFORE DELETE ON gitlab_job_generations
             WHEN EXISTS(SELECT 1 FROM gitlab_workflow_jobs WHERE id=OLD.job_record_id)
             BEGIN SELECT RAISE(ABORT,'GitLab job association is retained with its job'); END;
             CREATE TABLE gitlab_job_observations (
                id TEXT PRIMARY KEY NOT NULL, job_record_id TEXT NOT NULL,
                data_json TEXT NOT NULL, observed_at INTEGER NOT NULL
             );
             CREATE INDEX idx_gitlab_observation_job ON gitlab_job_observations(job_record_id,observed_at,id);"
        ).await?;
        Ok(())
    }
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("migrations are forward-only".into()))
    }
}
