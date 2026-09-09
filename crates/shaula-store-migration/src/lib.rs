//! Bundled forward migrations and schema checks for the Shaula ledger.
//! Migration failure must never leave a partly accepted control plane; the
//! daemon fails closed until pre/postconditions are satisfied.

pub mod m0001_fleet_tables;
pub mod m0002_template_tables;
pub mod m0003_auth_tables;
pub mod m0004_lifecycle_tables;
pub mod m0005_durable_format;
pub mod m0006_attestation_subject_verified;
pub mod m0007_http_state;
pub mod m0008_auth_multi_account;
pub mod m0009_auth_execution_contexts;
pub mod m0010_template_library;
pub mod m0011_listener_messages;
pub mod m0012_workflow_jobs;
pub mod m0013_operation_logs;
pub mod m0014_setup_info_capabilities;

use sea_orm_migration::prelude::*;

pub use sea_orm_migration::MigratorTrait;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m0001_fleet_tables::Migration),
            Box::new(m0002_template_tables::Migration),
            Box::new(m0003_auth_tables::Migration),
            Box::new(m0004_lifecycle_tables::Migration),
            Box::new(m0005_durable_format::Migration),
            Box::new(m0006_attestation_subject_verified::Migration),
            Box::new(m0007_http_state::Migration),
            Box::new(m0008_auth_multi_account::Migration),
            Box::new(m0009_auth_execution_contexts::Migration),
            Box::new(m0010_template_library::Migration),
            Box::new(m0011_listener_messages::Migration),
            Box::new(m0012_workflow_jobs::Migration),
            Box::new(m0013_operation_logs::Migration),
            Box::new(m0014_setup_info_capabilities::Migration),
        ]
    }
}

/// Runs pending migrations forward to the latest version atomically.
pub async fn migrate(db: &sea_orm::DatabaseConnection) -> Result<(), DbErr> {
    use sea_orm::TransactionTrait as _;
    use sea_orm_migration::MigratorTrait as _;

    // SeaORM does not wrap SQLite migrations in a transaction. DDL and
    // migration history must commit together so a failed upgrade is retryable.
    let tx = db.begin().await?;
    Migrator::up(&tx, None).await?;
    tx.commit().await
}
