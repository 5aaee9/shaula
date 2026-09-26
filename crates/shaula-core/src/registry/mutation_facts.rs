use super::ChangeView;

/// One accepted effective mutation with all its durable facts.
#[derive(Debug, Clone)]
pub struct MutationFacts {
    pub idempotency_operation: &'static str,
    pub authentication: super::AuthenticationContext,
    pub resource_kind: &'static str,
    pub resource_key: String,
    pub incarnation: String,
    pub revision: i64,
    /// Canonical spec JSON (fleet) or empty for decommission markers.
    pub spec_json: String,
    pub template: Option<(String, i64, String, String)>,
    pub template_pool: Vec<crate::template_pool::ResolvedTemplatePoolMember>,
    /// Shared-pool routing context frozen on fleet revisions (spec 0037
    /// §4): (pool key, pool revision). Unused by template/auth commits.
    pub template_pool_ref: Option<crate::template_pool::FleetPoolRef>,
    pub auth_desired: Option<(String, i64)>,
    pub inputs_digest: String,
    pub actor: String,
    pub now: i64,
    pub change: ChangeView,
    pub outbox_topic: String,
    pub outbox_payload: String,
    /// Idempotency record to store with the same transaction.
    pub idempotency: Option<(String, String, i32, String)>,
}
