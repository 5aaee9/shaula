// Iden enums extracted to keep the migration file under 400 lines.
use sea_orm_migration::prelude::*;

#[derive(DeriveIden)]
pub enum TemplateProfiles {
    Table,
    Key,
    Incarnation,
    DesiredRevision,
    ActiveRevision,
    ObservedRevision,
    ActiveAttestationId,
    Status,
    DeletionRequested,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
pub enum TemplateProfileRevisions {
    Table,
    Id,
    ProfileKey,
    Revision,
    ArtifactDigest,
    EngineRef,
    Platform,
    BindingsContract,
    ManifestJson,
    LockDigest,
    BindingsJson,
    BindingsDigest,
    FleetInputPolicyJson,
    State,
    Reason,
    CreatedAt,
}

#[derive(DeriveIden)]
pub enum TemplateConformanceAttestations {
    Table,
    Id,
    ProfileKey,
    Revision,
    SubjectJson,
    SubjectDigest,
    Result,
    EvidenceDigest,
    SuiteName,
    SuiteVersion,
    CompletedAt,
}

#[derive(DeriveIden)]
pub enum TemplateArtifacts {
    Table,
    Digest,
    SizeBytes,
    State,
    ReferenceCount,
    CreatedAt,
}

#[derive(DeriveIden)]
pub enum ProfileChanges {
    Table,
    Id,
    ResourceKind,
    ProfileKey,
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
