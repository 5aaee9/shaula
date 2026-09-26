//! No automatic legacy import or execution cutover during schema migration.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(
            "CREATE TABLE lifecycle_deployment (
                singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                deployment_id TEXT NOT NULL DEFAULT (lower(hex(randomblob(16)))),
                format_version INTEGER NOT NULL CHECK(format_version = 1),
                migration_required BOOLEAN NOT NULL DEFAULT 0 CHECK(migration_required IN (0, 1)),
                activated BOOLEAN NOT NULL DEFAULT 0 CHECK(activated IN (0, 1))
             );
             INSERT INTO lifecycle_deployment(singleton, format_version) VALUES (1, 1);
             CREATE TABLE lifecycle_workers (
                generation_id TEXT PRIMARY KEY NOT NULL
                    REFERENCES generation_http_state(generation_id) ON DELETE RESTRICT,
                control_hash BLOB NOT NULL CHECK(length(control_hash) = 32),
                phase TEXT NOT NULL CHECK(phase IN
                    ('admitted', 'launch_pending', 'running', 'fenced', 'completed', 'quarantined')),
                process_identity TEXT,
                cleanup_only BOOLEAN NOT NULL DEFAULT 0 CHECK(cleanup_only IN (0, 1)),
                handover TEXT,
                active_command TEXT,
                cleanup_revision INTEGER,
                protected_input BLOB,
                resource_state_seen BOOLEAN NOT NULL DEFAULT 0 CHECK(resource_state_seen IN (0, 1)),
                completion_request TEXT,
                completion_receipt TEXT,
                workspace_reaped BOOLEAN NOT NULL DEFAULT 0 CHECK(workspace_reaped IN (0, 1)),
                CHECK((completion_request IS NULL) = (completion_receipt IS NULL)),
                CHECK((phase = 'completed') = (completion_receipt IS NOT NULL))
             );
             CREATE TABLE lifecycle_fences (
                worker_attempt TEXT PRIMARY KEY NOT NULL,
                generation_id TEXT NOT NULL,
                worker_epoch INTEGER NOT NULL,
                process_identity TEXT,
                orphan_lock BLOB,
                confirmed_fenced BOOLEAN NOT NULL DEFAULT 0 CHECK(confirmed_fenced IN (0, 1)),
                outcome TEXT NOT NULL
             );
             CREATE TABLE lifecycle_imports (
                generation_id TEXT PRIMARY KEY NOT NULL
                    REFERENCES runner_generations(id) ON DELETE RESTRICT,
                commitment TEXT NOT NULL,
                classification TEXT NOT NULL CHECK(classification IN ('imported', 'quarantined', 'terminal')),
                receipt TEXT NOT NULL
             );"
        ).await?;
        Ok(())
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("migrations are forward-only".into()))
    }
}
