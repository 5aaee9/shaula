//! Durable per-epoch message, ACK and acquisition evidence.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE listener_messages (
                fleet_key TEXT NOT NULL,
                epoch INTEGER NOT NULL,
                message_id INTEGER NOT NULL,
                payload_digest TEXT NOT NULL,
                incarnation TEXT NOT NULL,
                fleet_revision INTEGER NOT NULL,
                mutation_fence INTEGER NOT NULL,
                profile_key TEXT NOT NULL,
                auth_revision INTEGER NOT NULL,
                context_json TEXT NOT NULL,
                acked INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY(fleet_key,epoch,message_id)
             );
             CREATE TABLE listener_acquisitions (
                fleet_key TEXT NOT NULL,
                epoch INTEGER NOT NULL,
                message_id INTEGER NOT NULL,
                runner_request_id INTEGER NOT NULL,
                state TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY(fleet_key,epoch,runner_request_id),
                FOREIGN KEY(fleet_key,epoch,message_id)
                    REFERENCES listener_messages(fleet_key,epoch,message_id)
             );
             CREATE INDEX idx_listener_acquisition_message
                ON listener_acquisitions(fleet_key,epoch,message_id,state);
             CREATE TABLE listener_job_observations (
                fleet_key TEXT NOT NULL, epoch INTEGER NOT NULL, message_id INTEGER NOT NULL,
                observation_kind TEXT NOT NULL, runner_request_id INTEGER NOT NULL,
                job_id TEXT NOT NULL, runner_name TEXT, observed_at INTEGER NOT NULL,
                PRIMARY KEY(fleet_key,epoch,message_id,observation_kind,runner_request_id),
                FOREIGN KEY(fleet_key,epoch,message_id)
                    REFERENCES listener_messages(fleet_key,epoch,message_id)
             );",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("migrations are forward-only".into()))
    }
}
