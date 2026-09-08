//! Database-owned immutable template archives and default source selections.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ArtifactArchives::Table)
                    .col(
                        ColumnDef::new(ArtifactArchives::Digest)
                            .string()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ArtifactArchives::Archive)
                            .binary()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ArtifactArchives::CreatedAt)
                            .big_integer()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(TemplateSources::Table)
                    .col(ColumnDef::new(TemplateSources::Key).string().primary_key())
                    .col(
                        ColumnDef::new(TemplateSources::ArtifactDigest)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(TemplateSources::Platform)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(TemplateSources::EngineRef)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(TemplateSources::CreatedAt)
                            .big_integer()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(TemplateSources::Table, TemplateSources::ArtifactDigest)
                            .to(ArtifactArchives::Table, ArtifactArchives::Digest),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("migrations are forward-only".to_string()))
    }
}

#[derive(DeriveIden)]
enum ArtifactArchives {
    Table,
    Digest,
    Archive,
    CreatedAt,
}
#[derive(DeriveIden)]
enum TemplateSources {
    Table,
    Key,
    ArtifactDigest,
    Platform,
    EngineRef,
    CreatedAt,
}
