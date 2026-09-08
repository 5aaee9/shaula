//! Multi-account GitHub authentication storage (spec 0011 §7): versioned
//! Target policy columns on auth revisions, frozen Account Bindings and
//! per-Fleet exact Resolved Auth Contexts. Pure schema migration: no GitHub
//! calls, no backfill of missing account/repository ids, and legacy rows
//! keep their exact legacy representation (schema_version defaults to 1).
//! The durable-format stamp advances to 2, so an older binary refuses the
//! data directory through the store format gate instead of misreading a
//! policy revision as a single-installation one.

use sea_orm_migration::prelude::*;

/// The durable-format version after this migration: databases holding v2
/// policy/bindings/context rows.
pub const DURABLE_FORMAT_VERSION_V2: i64 = 2;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Durable-format gate FIRST (spec 0011 §7): the 1→2 conversion is
        // legal only from the supported format-1 encoding. A format-0
        // database (deliberately unsupported pre-release identity
        // encodings, per m0005) must stay REJECTED and UNCHANGED — this
        // migration fails, the surrounding transaction rolls back every
        // DDL statement below, and the marker remains 0. There is no
        // conversion path for format 0.
        use sea_orm::ConnectionTrait;
        let marker: Option<i64> = manager
            .get_connection()
            .query_one(sea_orm::Statement::from_string(
                manager.get_connection().get_database_backend(),
                "SELECT version FROM durable_format",
            ))
            .await?
            .and_then(|row| row.try_get::<i64>("", "version").ok());
        match marker {
            Some(1) => {}
            Some(0) => {
                return Err(DbErr::Custom(
                    "durable format 0 (unsupported pre-release identity encoding) cannot be \
                     converted to the multi-account format; rebuild the data directory"
                        .to_string(),
                ));
            }
            other => {
                return Err(DbErr::Custom(format!(
                    "durable-format marker is {other:?}; expected 1 before the multi-account \
                     migration — refusing to upgrade"
                )));
            }
        }

        // Versioned Target policy + validation snapshot on the immutable
        // revision row. Existing rows keep schema_version 1.
        manager
            .alter_table(
                Table::alter()
                    .table(GithubAuthProfileRevisions::Table)
                    .add_column(
                        ColumnDef::new(GithubAuthProfileRevisions::SchemaVersion)
                            .big_integer()
                            .not_null()
                            .default(1),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(GithubAuthProfileRevisions::Table)
                    .add_column(
                        ColumnDef::new(GithubAuthProfileRevisions::PolicyJson)
                            .text()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(GithubAuthProfileRevisions::Table)
                    .add_column(
                        ColumnDef::new(GithubAuthProfileRevisions::ValidationSnapshotJson)
                            .text()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;

        // Frozen Account Bindings of one validated revision. Numeric
        // identity is authoritative; the login is display-only.
        manager
            .create_table(
                Table::create()
                    .table(GithubAuthRevisionBindings::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(GithubAuthRevisionBindings::Id)
                            .big_integer()
                            .primary_key()
                            .auto_increment(),
                    )
                    .col(
                        ColumnDef::new(GithubAuthRevisionBindings::ProfileKey)
                            .string_len(128)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(GithubAuthRevisionBindings::Revision)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(GithubAuthRevisionBindings::AccountId)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(GithubAuthRevisionBindings::AccountKind)
                            .string_len(16)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(GithubAuthRevisionBindings::Login)
                            .string_len(256)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(GithubAuthRevisionBindings::InstallationId)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(GithubAuthRevisionBindings::RepositorySelection)
                            .string_len(16)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(GithubAuthRevisionBindings::ValidatedAtMs)
                            .big_integer()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_auth_bindings_key_rev_account")
                    .table(GithubAuthRevisionBindings::Table)
                    .col(GithubAuthRevisionBindings::ProfileKey)
                    .col(GithubAuthRevisionBindings::Revision)
                    .col(GithubAuthRevisionBindings::AccountId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Exact desired/observed Resolved Auth Context per fleet. Both
        // sides carry the full ref tuple; a ref match with a drifted
        // context is detectable and never auto-overwritten.
        manager
            .create_table(
                Table::create()
                    .table(FleetAuthContexts::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(FleetAuthContexts::FleetKey)
                            .string_len(128)
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(FleetAuthContexts::DesiredProfileKey)
                            .string_len(128)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(FleetAuthContexts::DesiredRevision)
                            .big_integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(FleetAuthContexts::DesiredFence)
                            .big_integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(FleetAuthContexts::DesiredContextJson)
                            .text()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(FleetAuthContexts::ObservedProfileKey)
                            .string_len(128)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(FleetAuthContexts::ObservedRevision)
                            .big_integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(FleetAuthContexts::ObservedContextJson)
                            .text()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(FleetAuthContexts::State)
                            .string_len(32)
                            .not_null()
                            .default("Pending"),
                    )
                    .col(
                        ColumnDef::new(FleetAuthContexts::Reason)
                            .string_len(128)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(FleetAuthContexts::Attempts)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(FleetAuthContexts::NextRetryAt)
                            .big_integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(FleetAuthContexts::UpdatedAt)
                            .big_integer()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // Advance the durable-format stamp: from this migration onward the
        // database may hold v2 policy/binding/context rows. The gate above
        // already proved the marker was the supported format 1.
        manager
            .get_connection()
            .execute_unprepared("UPDATE durable_format SET version = 2 WHERE version = 1")
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("migrations are forward-only".to_string()))
    }
}

#[derive(DeriveIden)]
enum GithubAuthProfileRevisions {
    Table,
    SchemaVersion,
    PolicyJson,
    ValidationSnapshotJson,
}

#[derive(DeriveIden)]
enum GithubAuthRevisionBindings {
    Table,
    Id,
    ProfileKey,
    Revision,
    AccountId,
    AccountKind,
    Login,
    InstallationId,
    RepositorySelection,
    ValidatedAtMs,
}

#[derive(DeriveIden)]
enum FleetAuthContexts {
    Table,
    FleetKey,
    DesiredProfileKey,
    DesiredRevision,
    DesiredFence,
    DesiredContextJson,
    ObservedProfileKey,
    ObservedRevision,
    ObservedContextJson,
    State,
    Reason,
    Attempts,
    NextRetryAt,
    UpdatedAt,
}
