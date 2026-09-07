//! Entity→row mapping helpers, split to keep files within 400 lines.

use shaula_core::registry::{AuthRevisionRow, FleetHead, ProfileHead, TemplateRevisionRow};

pub(crate) fn fleet_head(f: crate::entities::fleet::fleets::Model) -> FleetHead {
    FleetHead {
        key: f.key,
        incarnation: f.incarnation,
        desired_revision: f.desired_revision,
        observed_revision: f.observed_revision,
        tombstone: f.tombstone,
        deletion_marker: f.deletion_marker,
        phase: f.phase,
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
        kind: r.kind,
        app_id: r.app_id,
        installation_id: r.installation_id,
        pat_principal: r.pat_principal,
        allowlist_json: r.allowlist_json,
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
