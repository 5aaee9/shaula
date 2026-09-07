//! Durable-format marker (R8-03): the persisted identity encoding —
//! hash tuple encoding, attestation record ids, canonical subject JSON —
//! is VERSIONED. A database holding rows from an older pre-release build
//! is stamped LEGACY (0) instead of silently resolving identities under
//! a different encoding than the one that wrote them; `Store::migrate`
//! then FAILS CLOSED and the operator must rebuild the data directory.
//! There is deliberately no in-place conversion: replaying the ambiguous
//! legacy encodings is unsafe (see the R7-05/R8-03 review trail).

use sea_orm_migration::prelude::*;

/// The durable identity encoding version this build reads and writes.
/// Bump ONLY together with a deliberate, reviewed durable-format change.
pub const DURABLE_FORMAT_VERSION: i64 = 1;

/// The version stamped on databases that already contain rows from a
/// pre-release build; startup refuses them.
pub const DURABLE_FORMAT_LEGACY: i64 = 0;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(DurableFormat::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(DurableFormat::Version)
                            .big_integer()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;
        // A database that ALREADY holds durable rows was written by a
        // pre-release build with the legacy identity encoding: stamp it
        // LEGACY so startup fails closed. A fresh database is stamped
        // with the current version.
        manager
            .get_connection()
            .execute_unprepared(
                "INSERT INTO durable_format (version) \
                 SELECT CASE WHEN EXISTS (SELECT 1 FROM template_profiles) \
                             OR EXISTS (SELECT 1 FROM github_auth_profiles) \
                             OR EXISTS (SELECT 1 FROM fleets) \
                            THEN 0 \
                            ELSE 1 END",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("migrations are forward-only".to_string()))
    }
}

#[derive(DeriveIden)]
enum DurableFormat {
    Table,
    Version,
}
