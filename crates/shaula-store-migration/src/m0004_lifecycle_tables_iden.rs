// Iden enums extracted to keep the migration file under 400 lines.
use sea_orm_migration::prelude::*;

#[derive(DeriveIden)]
pub enum RunnerGenerations {
    Table,
    Id,
    FleetKey,
    RunnerName,
    GenerationName,
    FleetRevision,
    TemplateProfileKey,
    TemplateRevision,
    TemplateArtifactDigest,
    AttestationId,
    InputsDigest,
    State,
    Subphase,
    JitPhase,
    GithubRunnerId,
    WorkspacePath,
    ShaulaResultJson,
    ShaulaResultDigest,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
pub enum RunnerOperations {
    Table,
    Id,
    GenerationId,
    Kind,
    State,
    Attempts,
    NextRetryAt,
    SavedPlanPath,
    SavedPlanDigest,
    ProvenanceJson,
    LeaseOwner,
    LeaseExpiresAt,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
pub enum FleetSessions {
    Table,
    FleetKey,
    SessionId,
    Epoch,
    ScaleSetId,
    MessageQueueUrl,
    QueueToken,
    LastMessageId,
    CreatedAt,
}

#[derive(DeriveIden)]
pub enum FleetDemand {
    Table,
    FleetKey,
    TotalAssignedJobs,
    UpdatedAt,
}

#[derive(DeriveIden)]
pub enum JobObservations {
    Table,
    Id,
    FleetKey,
    MessageId,
    ObservationKind,
    RunnerRequestId,
    JobId,
    RunnerName,
    ObservedAt,
}

#[derive(DeriveIden)]
pub enum AcquisitionIntents {
    Table,
    Id,
    FleetKey,
    MessageId,
    RunnerRequestId,
    Epoch,
    State,
    CreatedAt,
}

#[derive(DeriveIden)]
pub enum ScaleSetState {
    Table,
    FleetKey,
    ScaleSetId,
    Name,
    RunnerGroup,
    Fingerprint,
    State,
    AttemptId,
    CreatedAt,
    UpdatedAt,
}
