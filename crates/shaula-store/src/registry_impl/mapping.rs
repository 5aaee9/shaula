//! Entity→row mapping helpers, split to keep files within 400 lines.

use shaula_core::registry::{
    AuthHandoffRow, AuthRevisionRow, ChangeView, FleetHead, ProfileHead, TemplateRevisionRow,
};

pub(crate) fn fleet_head(f: crate::entities::fleet::fleets::Model) -> FleetHead {
    FleetHead {
        key: f.key,
        incarnation: f.incarnation,
        desired_revision: f.desired_revision,
        observed_revision: f.observed_revision,
        tombstone: f.tombstone,
        deletion_marker: f.deletion_marker,
        phase: f.phase,
        mutation_fence: f.mutation_fence,
    }
}

pub(crate) fn fleet_revision_row(
    r: crate::entities::fleet::fleet_revisions::Model,
) -> shaula_core::registry::store_port::FleetRevisionRow {
    shaula_core::registry::store_port::FleetRevisionRow {
        fleet_key: r.fleet_key,
        revision: r.revision,
        spec_json: r.spec_json,
        template_profile_key: r.template_profile_key,
        template_revision: r.template_revision,
        template_artifact_digest: r.template_artifact_digest,
        template_attestation_id: r.template_attestation_id,
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
        platform: r.platform,
        bindings_contract: r.bindings_contract,
        state: r.state,
        reason: crate::template_activation::public_validation_reason(r.reason),
        bindings_present: r.bindings_json.is_some(),
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
) -> AuthHandoffRow {
    AuthHandoffRow {
        fleet_key: h.fleet_key,
        desired: (h.desired_profile_key, h.desired_revision),
        observed: h
            .observed_profile_key
            .map(|k| (k, h.observed_revision.unwrap_or_default())),
        state: h.state,
        cleanup_only: h.cleanup_only,
        blocked_reason: h.reason.clone(),
        retry_at: h.next_retry_at,
    }
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
