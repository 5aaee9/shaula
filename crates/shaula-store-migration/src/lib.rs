//! Bundled forward migrations and schema checks for the Shaula ledger.
//! Migration failure must never leave a partly accepted control plane; the
//! daemon fails closed until pre/postconditions are satisfied.

pub mod m0001_fleet_tables;
pub mod m0002_template_tables;
pub mod m0003_auth_tables;
pub mod m0004_lifecycle_tables;
pub mod m0005_durable_format;
pub mod m0006_attestation_subject_verified;

use sea_orm_migration::prelude::*;

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
        ]
    }
}

/// Runs pending migrations forward to the latest version.
pub async fn migrate(db: &sea_orm::DatabaseConnection) -> Result<(), DbErr> {
    use sea_orm_migration::MigratorTrait as _;
    Migrator::up(db, None).await
}
