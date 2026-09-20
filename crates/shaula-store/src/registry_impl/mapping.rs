//! Entity→row mapping helpers, split to keep files within 400 lines.

use shaula_core::registry::{
    AuthHandoffRow, AuthRevisionRow, ChangeView, FleetHead, ProfileHead, TemplateRevisionRow,
};
use shaula_core::template_pool::{PoolFailurePolicy, ResolvedTemplatePoolMember};

use crate::store::{StoreError, StoreResult};

pub(crate) fn template_inputs_from_json(
    json: &str,
) -> StoreResult<serde_json::Map<String, serde_json::Value>> {
    serde_json::from_str(json).map_err(|error| {
        StoreError::Corrupt(format!("template pool member inputs are invalid: {error}"))
    })
}

pub(crate) fn pool_member_weight(weight: i64) -> StoreResult<u32> {
    u32::try_from(weight).map_err(|_| {
        StoreError::Corrupt(format!(
            "template pool member weight is out of range: {weight}"
        ))
    })
}

pub(crate) fn pool_failure_policy(value: &str) -> StoreResult<PoolFailurePolicy> {
    match value {
        "backpressure" => Ok(PoolFailurePolicy::Backpressure),
        "redistribute" => Ok(PoolFailurePolicy::Redistribute),
        other => Err(StoreError::Corrupt(format!(
            "template pool failure policy is invalid: {other}"
        ))),
    }
}

struct RawPoolMember {
    member_key: String,
    template_profile_key: String,
    template_revision: i64,
    template_artifact_digest: String,
    template_attestation_id: String,
    template_inputs_json: String,
    inputs_digest: String,
    weight: i64,
    max_runners: Option<i64>,
}

fn resolved_pool_member(member: RawPoolMember) -> StoreResult<ResolvedTemplatePoolMember> {
    Ok(ResolvedTemplatePoolMember {
        key: member.member_key,
        template_profile_key: member.template_profile_key,
        template_revision: member.template_revision,
        template_artifact_digest: member.template_artifact_digest,
        template_attestation_id: member.template_attestation_id,
        template_inputs: template_inputs_from_json(&member.template_inputs_json)?,
        inputs_digest: member.inputs_digest,
        weight: pool_member_weight(member.weight)?,
        max_runners: member.max_runners,
    })
}

pub(crate) fn pool_member_row(
    m: crate::entities::template_pool::template_pool_members::Model,
) -> StoreResult<ResolvedTemplatePoolMember> {
    resolved_pool_member(RawPoolMember {
        member_key: m.member_key,
        template_profile_key: m.template_profile_key,
        template_revision: m.template_revision,
        template_artifact_digest: m.template_artifact_digest,
        template_attestation_id: m.template_attestation_id,
        template_inputs_json: m.template_inputs_json,
        inputs_digest: m.inputs_digest,
        weight: m.weight,
        max_runners: m.max_runners,
    })
}

pub(crate) fn fleet_pool_member_row(
    m: crate::entities::fleet::fleet_revision_pool_members::Model,
) -> StoreResult<ResolvedTemplatePoolMember> {
    resolved_pool_member(RawPoolMember {
        member_key: m.member_key,
        template_profile_key: m.template_profile_key,
        template_revision: m.template_revision,
        template_artifact_digest: m.template_artifact_digest,
        template_attestation_id: m.template_attestation_id,
        template_inputs_json: m.template_inputs_json,
        inputs_digest: m.inputs_digest,
        weight: m.weight,
        max_runners: m.max_runners,
    })
}

pub(crate) fn fleet_head(f: crate::entities::fleet::fleets::Model) -> FleetHead {
    FleetHead {
        key: f.key,
        incarnation: f.incarnation,
        desired_revision: f.desired_revision,
        observed_revision: f.observed_revision,
        tombstone: f.tombstone,
        deletion_marker: f.deletion_marker,
        phase: f.phase,
        last_condition_reason: f.last_condition_reason,
        mutation_fence: f.mutation_fence,
    }
}

pub(crate) fn fleet_revision_row(
    r: crate::entities::fleet::fleet_revisions::Model,
) -> shaula_core::registry::store_port::FleetRevisionRow {
    let template_pool = Vec::new();
    shaula_core::registry::store_port::FleetRevisionRow {
        fleet_key: r.fleet_key,
        revision: r.revision,
        spec_json: r.spec_json,
        template_profile_key: r.template_profile_key,
        template_revision: r.template_revision,
        template_artifact_digest: r.template_artifact_digest,
        template_attestation_id: r.template_attestation_id,
        template_pool,
        template_pool_ref: r.template_pool_ref.clone().zip(r.template_pool_revision),
        auth_desired: (r.auth_desired_profile_key, r.auth_desired_revision),
        inputs_digest: r.inputs_digest,
        created_at: r.created_at,
    }
}

pub(crate) fn auth_row(
    r: crate::entities::auth::github_auth_profile_revisions::Model,
) -> AuthRevisionRow {
    AuthRevisionRow {
        profile_key: r.profile_key,
        revision: r.revision,
        state: r.state,
        reason: r.reason,
        kind: r.kind,
        app_id: r.app_id,
        schema_version: r.schema_version,
        policy_json: r.policy_json,
        validation_snapshot_json: r.validation_snapshot_json,
    }
}

pub(crate) fn template_row(
    r: crate::entities::template::template_profile_revisions::Model,
) -> TemplateRevisionRow {
    TemplateRevisionRow {
        profile_key: r.profile_key,
        revision: r.revision,
        artifact_digest: r.artifact_digest,
        engine_ref: r.engine_ref,
        source_key: r.source_key,
        platform: r.platform,
        bindings_contract: r.bindings_contract,
        state: r.state,
        reason: crate::template_activation::public_validation_reason(r.reason),
        bindings_present: r.bindings_json.is_some(),
        // Protected-memory seam: consumed only by the schema-driven
        // projection (spec 0038), never serialized into a response.
        bindings_json: r.bindings_json,
        bindings_digest: r.bindings_digest,
        fleet_input_policy_json: r.fleet_input_policy_json,
    }
}

pub(crate) fn tpl_profile_head(
    p: crate::entities::template::template_profiles::Model,
) -> ProfileHead {
    ProfileHead {
        key: p.key,
        incarnation: p.incarnation,
        desired_revision: p.desired_revision,
        active_revision: p.active_revision,
        active_attestation_id: p.active_attestation_id,
        status: p.status,
    }
}

pub(crate) fn auth_profile_head(
    p: crate::entities::auth::github_auth_profiles::Model,
) -> ProfileHead {
    ProfileHead {
        key: p.key,
        incarnation: p.incarnation,
        desired_revision: p.desired_revision,
        active_revision: p.active_revision,
        active_attestation_id: None,
        status: p.status,
    }
}

pub(crate) fn auth_handoff_row(
    h: crate::entities::fleet::fleet_auth_handoffs::Model,
) -> StoreResult<AuthHandoffRow> {
    let observed = match (h.observed_profile_key, h.observed_revision) {
        (Some(profile_key), Some(revision)) => Some((profile_key, revision)),
        (Some(_), None) => {
            return Err(StoreError::Corrupt(
                "auth handoff has an observed profile without a revision".into(),
            ))
        }
        (None, Some(_)) => {
            return Err(StoreError::Corrupt(
                "auth handoff has an observed revision without a profile".into(),
            ))
        }
        (None, None) => None,
    };
    Ok(AuthHandoffRow {
        fleet_key: h.fleet_key,
        desired: (h.desired_profile_key, h.desired_revision),
        observed,
        state: h.state,
        cleanup_only: h.cleanup_only,
        blocked_reason: h.reason.clone(),
        retry_at: h.next_retry_at,
    })
}

pub(crate) fn fleet_change_row(c: crate::entities::fleet::fleet_changes::Model) -> ChangeView {
    ChangeView {
        id: c.id,
        resource_kind: "fleet".to_string(),
        resource_key: c.fleet_key,
        revision: c.revision,
        kind: c.kind,
        state: c.state,
        reason: c.reason,
    }
}

pub(crate) fn profile_change_row(
    c: crate::entities::template::profile_changes::Model,
) -> ChangeView {
    ChangeView {
        id: c.id,
        resource_kind: c.resource_kind,
        resource_key: c.profile_key,
        revision: c.revision.unwrap_or_default(),
        kind: c.kind,
        state: c.state,
        reason: c.reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_pool_fields_reject_corrupt_values() {
        assert!(matches!(
            template_inputs_from_json("not-json"),
            Err(StoreError::Corrupt(_))
        ));
        assert!(matches!(
            pool_member_weight(-1),
            Err(StoreError::Corrupt(_))
        ));
        assert!(matches!(
            pool_failure_policy("unknown"),
            Err(StoreError::Corrupt(_))
        ));
    }

    #[test]
    fn persisted_pool_fields_keep_valid_values() {
        let inputs = template_inputs_from_json(r#"{"size":"standard"}"#);
        assert_eq!(
            inputs
                .as_ref()
                .ok()
                .and_then(|value| value.get("size"))
                .and_then(serde_json::Value::as_str),
            Some("standard")
        );
        assert_eq!(pool_member_weight(7).ok(), Some(7));
        assert_eq!(
            pool_failure_policy("redistribute").ok(),
            Some(PoolFailurePolicy::Redistribute)
        );
    }
}
