//! R10-05: the durable attestation record must carry the SUBJECT
//! VERIFICATION verdict itself — audit-only evidence is weaker than the
//! spec requires (spec 0005 §5/§6: a mismatched attestation stays durable
//! evidence with `subject_verified = false`). Adds the missing column.
//! Legacy rows (pre-release data dirs) are refused by the durable-format
//! marker in m0005 before this matters; the default keeps fresh rows
//! consistent.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(TemplateConformanceAttestations::Table)
                    .add_column_if_not_exists(
                        ColumnDef::new(TemplateConformanceAttestations::SubjectVerified)
                            .boolean()
                            .not_null()
                            .default(true),
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
enum TemplateConformanceAttestations {
    Table,
    SubjectVerified,
}
