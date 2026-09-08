//! Write records shared by the registry's persistence ports.

/// Parameter object for recording a runner operation.
#[derive(Debug, Clone)]
pub struct OperationInsert {
    pub id: String,
    pub generation_id: String,
    pub kind: String,
    pub state: String,
    pub provenance_json: Option<String>,
    pub saved_plan_path: Option<String>,
    pub saved_plan_digest: Option<String>,
    pub now: i64,
}

/// Parameter object for the shared idempotency record.
#[derive(Debug, Clone)]
pub struct IdempotencyInsert {
    pub id: String,
    pub resource_kind: String,
    pub resource_key: String,
    pub idempotency_key: String,
    pub request_hash: String,
    pub response_status: i32,
    pub response_body: Option<String>,
    pub now: i64,
}

/// Parameter object for appending one audit fact.
#[derive(Debug, Clone)]
pub struct AuditAppend {
    pub resource_kind: String,
    pub action: String,
    pub actor: String,
    pub resource_key: String,
    pub revision: Option<i64>,
    pub outcome: String,
    pub detail_json: Option<String>,
    pub now: i64,
}

/// Parameter object for one Profile Change row.
#[derive(Debug, Clone)]
pub struct ProfileChangeInsert {
    pub id: String,
    pub resource_kind: String,
    pub profile_key: String,
    pub revision: Option<i64>,
    pub kind: String,
    pub now: i64,
}

/// Parameter object for one idempotent job observation.
#[derive(Debug, Clone)]
pub struct JobObservationInsert {
    pub fleet_key: String,
    pub message_id: i64,
    pub observation_kind: String,
    pub runner_request_id: i64,
    pub job_id: String,
    pub runner_name: Option<String>,
    pub now: i64,
}
