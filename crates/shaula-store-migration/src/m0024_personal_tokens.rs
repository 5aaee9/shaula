use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(
            "CREATE TABLE personal_access_tokens (
                id TEXT PRIMARY KEY NOT NULL, owner TEXT NOT NULL, created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL, revoked_at INTEGER, last_used_at INTEGER,
                record_json TEXT NOT NULL);
             CREATE INDEX idx_pat_owner ON personal_access_tokens(owner, created_at DESC, id DESC);
             CREATE TABLE personal_token_issues (
                owner TEXT NOT NULL, idempotency_key TEXT NOT NULL, request_hash TEXT NOT NULL,
                token_id TEXT NOT NULL REFERENCES personal_access_tokens(id), created_at INTEGER NOT NULL,
                PRIMARY KEY(owner, idempotency_key));
             CREATE INDEX idx_pat_issue_rate ON personal_token_issues(created_at, owner);"
        ).await?;
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("migrations are forward-only".into()))
    }
}
