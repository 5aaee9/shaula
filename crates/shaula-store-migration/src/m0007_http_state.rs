//! New generations opt in atomically. Never backfill legacy local-state
//! generations with empty HTTP state or discard their operation evidence.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(
            "CREATE TABLE generation_http_state (
                generation_id TEXT PRIMARY KEY NOT NULL
                    REFERENCES runner_generations(id) ON DELETE RESTRICT,
                worker_epoch INTEGER NOT NULL CHECK(worker_epoch > 0),
                worker_attempt TEXT NOT NULL,
                capability_hash BLOB NOT NULL CHECK(length(capability_hash) = 32),
                revoked BOOLEAN NOT NULL DEFAULT 0 CHECK(revoked IN (0, 1)),
                create_started BOOLEAN NOT NULL DEFAULT 0 CHECK(create_started IN (0, 1)),
                sealed BOOLEAN NOT NULL DEFAULT 0 CHECK(sealed IN (0, 1)),
                revision INTEGER NOT NULL DEFAULT 0 CHECK(revision >= 0),
                state_bytes BLOB,
                lineage TEXT,
                serial INTEGER CHECK(serial >= 0),
                lock_id TEXT,
                lock_info BLOB,
                CHECK ((lock_id IS NULL) = (lock_info IS NULL)),
                CHECK ((revision = 0 AND state_bytes IS NULL AND lineage IS NULL AND serial IS NULL)
                    OR (revision > 0 AND state_bytes IS NOT NULL AND lineage IS NOT NULL AND serial IS NOT NULL)),
                CHECK (sealed = 0 OR (revision > 0 AND lock_id IS NULL))
            )",
        ).await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("migrations are forward-only".to_string()))
    }
}
