// Iden enums extracted to keep the migration file under 400 lines.
use sea_orm_migration::prelude::*;

#[derive(DeriveIden)]
pub enum Fleets {
    Table,
    Key,
    Incarnation,
    DesiredRevision,
    ObservedRevision,
    MutationFence,
    DeletionMarker,
    Phase,
    Tombstone,
    LastConditionReason,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
pub enum FleetRevisions {
    Table,
    Id,
    FleetKey,
    Incarnation,
    Revision,
    SpecJson,
    TemplateProfileKey,
    TemplateRevision,
    TemplateArtifactDigest,
    TemplateAttestationId,
    AuthDesiredProfileKey,
    AuthDesiredRevision,
    InputsDigest,
    Actor,
    CreatedAt,
}

#[derive(DeriveIden)]
pub enum FleetAuthHandoffs {
    Table,
    FleetKey,
    DesiredProfileKey,
    DesiredRevision,
    ObservedProfileKey,
    ObservedRevision,
    State,
    CleanupOnly,
    Attempts,
    NextRetryAt,
    LeaseOwner,
    LeaseExpiresAt,
    Reason,
}

#[derive(DeriveIden)]
pub enum FleetChanges {
    Table,
    Id,
    FleetKey,
    Revision,
    Kind,
    State,
    Attempts,
    NextRetryAt,
    LeaseOwner,
    LeaseExpiresAt,
    Reason,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
pub enum IdempotencyRecords {
    Table,
    Id,
    ResourceKind,
    ResourceKey,
    IdempotencyKey,
    RequestHash,
    ResponseStatus,
    ResponseBody,
    CreatedAt,
}

#[derive(DeriveIden)]
pub enum AuditRecords {
    Table,
    Seq,
    ResourceKind,
    Action,
    Actor,
    ResourceKey,
    Revision,
    Outcome,
    DetailJson,
    CreatedAt,
}

#[derive(DeriveIden)]
pub enum Outbox {
    Table,
    Id,
    ResourceKind,
    Topic,
    Payload,
    CreatedAt,
    ProcessedAt,
}
