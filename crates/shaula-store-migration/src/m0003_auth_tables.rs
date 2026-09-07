//! GitHub Auth Profile tables: profiles and immutable credential revisions.
//! Credential plaintext bytes live here by accepted decision (ADR-0007,
//! ADR-0009); the whole database is credential-grade.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(GithubAuthProfiles::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(GithubAuthProfiles::Key)
                            .string_len(128)
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(GithubAuthProfiles::Incarnation)
                            .string_len(64)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(GithubAuthProfiles::DesiredRevision)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(GithubAuthProfiles::ActiveRevision)
                            .big_integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(GithubAuthProfiles::ObservedRevision)
                            .big_integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(GithubAuthProfiles::Status)
                            .string_len(32)
                            .not_null()
                            .default("Pending"),
                    )
                    .col(
                        ColumnDef::new(GithubAuthProfiles::DeletionRequested)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(GithubAuthProfiles::CreatedAt)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(GithubAuthProfiles::UpdatedAt)
                            .big_integer()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(GithubAuthProfileRevisions::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(GithubAuthProfileRevisions::Id)
                            .big_integer()
                            .primary_key()
                            .auto_increment(),
                    )
                    .col(
                        ColumnDef::new(GithubAuthProfileRevisions::ProfileKey)
                            .string_len(128)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(GithubAuthProfileRevisions::Revision)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(GithubAuthProfileRevisions::Kind)
                            .string_len(32)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(GithubAuthProfileRevisions::AppId)
                            .string_len(64)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(GithubAuthProfileRevisions::InstallationId)
                            .big_integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(GithubAuthProfileRevisions::PatPrincipal)
                            .string_len(256)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(GithubAuthProfileRevisions::AllowlistJson)
                            .text()
                            .not_null(),
                    )
                    // Plaintext credential bytes (PAT or App private key).
                    .col(
                        ColumnDef::new(GithubAuthProfileRevisions::CredentialBytes)
                            .blob()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(GithubAuthProfileRevisions::State)
                            .string_len(32)
                            .not_null()
                            .default("Pending"),
                    )
                    .col(
                        ColumnDef::new(GithubAuthProfileRevisions::Reason)
                            .string_len(128)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(GithubAuthProfileRevisions::CreatedAt)
                            .big_integer()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_auth_revisions_key_rev")
                    .table(GithubAuthProfileRevisions::Table)
                    .col(GithubAuthProfileRevisions::ProfileKey)
                    .col(GithubAuthProfileRevisions::Revision)
                    .unique()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("migrations are forward-only".to_string()))
    }
}

#[derive(DeriveIden)]
enum GithubAuthProfiles {
    Table,
    Key,
    Incarnation,
    DesiredRevision,
    ActiveRevision,
    ObservedRevision,
    Status,
    DeletionRequested,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum GithubAuthProfileRevisions {
    Table,
    Id,
    ProfileKey,
    Revision,
    Kind,
    AppId,
    InstallationId,
    PatPrincipal,
    AllowlistJson,
    CredentialBytes,
    State,
    Reason,
    CreatedAt,
}
