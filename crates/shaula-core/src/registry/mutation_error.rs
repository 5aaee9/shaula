/// Mutation outcomes used by the HTTP adapter to map status codes.
#[derive(Debug, Clone, PartialEq)]
pub enum MutationError {
    /// 400: a new PUT cannot be both create-only and replacement-only.
    /// Accepted historical requests are replayed before this classification.
    ConflictingPreconditions,
    /// 428
    PreconditionRequired,
    /// 412 with current revision metadata
    PreconditionFailed { current: (String, i64) },
    /// 409 immutable identity change
    IdentityConflict,
    /// 409 idempotency key reuse with different content
    IdempotencyConflict,
    /// Historical request has no verified principal.
    LegacyIdempotencyConflict,
    /// 422 inadmissible spec
    Unprocessable {
        reason: crate::error::ReasonCode,
        summary: String,
    },
    /// 404
    NotFound,
    /// 410 terminal tombstone
    Gone { tombstone: String },
    /// 409 referenced resources block retirement; stays visibly blocked
    RetirementBlocked { reason: String },
    /// 409 the durable state refuses this transition (e.g. spec 0028
    /// finalize on a non-Quarantined generation)
    Conflict { summary: String },
    /// 429 admission/backlog limit
    TooManyRequests { retry_after_secs: u64 },
}
