//! Fleet control-plane tables: fleets, revisions, auth handoffs, changes,
//! idempotency, audit and outbox. The audit/idempotency/outbox tables carry
//! a `resource_kind` discriminator so Fleet and Profile mutations share one
//! durable shape.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let _db = manager.get_connection();

        manager
            .create_table(
                Table::create()
                    .table(Fleets::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Fleets::Key).string_len(128).primary_key())
                    .col(
                        ColumnDef::new(Fleets::Incarnation)
                            .string_len(64)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Fleets::DesiredRevision)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(Fleets::ObservedRevision)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(Fleets::MutationFence)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(Fleets::DeletionMarker)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(Fleets::Phase)
                            .string_len(32)
                            .not_null()
                            .default("Pending"),
                    )
                    .col(
                        ColumnDef::new(Fleets::Tombstone)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(Fleets::LastConditionReason)
                            .string_len(128)
                            .null(),
                    )
                    .col(ColumnDef::new(Fleets::CreatedAt).big_integer().not_null())
                    .col(ColumnDef::new(Fleets::UpdatedAt).big_integer().not_null())
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(FleetRevisions::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(FleetRevisions::Id)
                            .big_integer()
                            .primary_key()
                            .auto_increment(),
                    )
                    .col(
                        ColumnDef::new(FleetRevisions::FleetKey)
                            .string_len(128)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(FleetRevisions::Incarnation)
                            .string_len(64)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(FleetRevisions::Revision)
                            .big_integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(FleetRevisions::SpecJson).text().not_null())
                    .col(
                        ColumnDef::new(FleetRevisions::TemplateProfileKey)
                            .string_len(128)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(FleetRevisions::TemplateRevision)
                            .big_integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(FleetRevisions::TemplateArtifactDigest)
                            .string_len(128)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(FleetRevisions::TemplateAttestationId)
                            .string_len(128)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(FleetRevisions::AuthDesiredProfileKey)
                            .string_len(128)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(FleetRevisions::AuthDesiredRevision)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(FleetRevisions::InputsDigest)
                            .string_len(128)
                            .not_null(),
                    )
                    .col(ColumnDef::new(FleetRevisions::Actor).string_len(256).null())
                    .col(
                        ColumnDef::new(FleetRevisions::CreatedAt)
                            .big_integer()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_fleet_revisions_key_rev")
                    .table(FleetRevisions::Table)
                    .col(FleetRevisions::FleetKey)
                    .col(FleetRevisions::Revision)
                    .unique()
                    .to_owned(),
            )
            .await?;

        shared_tables::create_remaining(manager).await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("migrations are forward-only".to_string()))
    }
}

use self::iden::*;

#[path = "m0001_fleet_tables_iden.rs"]
mod iden;

#[path = "m0001_shared_tables.rs"]
mod shared_tables;
