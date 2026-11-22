//! Fleet spec validation shared by create, replace and decommission paths.

use super::{is_placeholder_github, FleetProviderKind, FleetSpec};
use crate::auth::validate_profile_key_for_fleet_ref;
use crate::error::{CoreError, CoreResult, ReasonCode};

/// Canonical validation shared by create, replace and decommission paths.
pub fn validate_fleet_spec(spec: &FleetSpec) -> CoreResult<()> {
    // Exactly one template source: a profile ref, a shared pool ref, or the
    // legacy inline pool. The three are mutually exclusive.
    let pool_ref = spec
        .template_pool_ref
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty());
    let sources = [
        !spec.template_profile_ref.is_empty(),
        pool_ref.is_some(),
        spec.template_pool.is_some(),
    ]
    .iter()
    .filter(|present| **present)
    .count();
    if sources > 1 {
        return Err(CoreError::new(
            ReasonCode::SpecInvalid,
            "template_profile_ref, template_pool_ref and template_pool are mutually exclusive",
        ));
    }
    if sources == 0 {
        return Err(CoreError::new(
            ReasonCode::SpecInvalid,
            "one of template_profile_ref, template_pool_ref or template_pool is required",
        ));
    }
    if let Some(pool) = &spec.template_pool {
        pool.validate()?;
    }
    if let Some(pool_ref) = pool_ref {
        if pool_ref.len() > 128 {
            return Err(CoreError::new(
                ReasonCode::SpecInvalid,
                "template_pool_ref must be a template pool key",
            ));
        }
    }
    if (spec.template_pool.is_some() || spec.template_pool_ref.is_some())
        && !spec.template_inputs.is_empty()
    {
        return Err(CoreError::new(
            ReasonCode::SpecInvalid,
            "template_inputs must be empty when template_pool is declared",
        ));
    }
    match spec.kind {
        FleetProviderKind::Github => {
            if spec.forgejo.is_some() || spec.gitlab.is_some() {
                return Err(CoreError::new(
                    ReasonCode::SpecInvalid,
                    "github fleets must not declare a forgejo or gitlab section",
                ));
            }
            if spec.github.scale_set_name.trim().is_empty()
                || spec.github.scale_set_name.len() > 100
            {
                return Err(CoreError::new(
                    ReasonCode::SpecInvalid,
                    "scale_set_name must be 1..=100 characters",
                ));
            }
            if spec.github.runner_group.trim().is_empty() || spec.github.runner_group.len() > 100 {
                return Err(CoreError::new(
                    ReasonCode::SpecInvalid,
                    "runner_group must be 1..=100 characters",
                ));
            }
            validate_labels(&spec.github.labels)?;
            validate_profile_key_for_fleet_ref(&spec.github.auth_profile_ref)?;
        }
        FleetProviderKind::Forgejo => {
            if !is_placeholder_github(&spec.github) || spec.gitlab.is_some() {
                return Err(CoreError::new(
                    ReasonCode::SpecInvalid,
                    "forgejo fleets must not declare a github or gitlab section",
                ));
            }
            let Some(forgejo) = &spec.forgejo else {
                return Err(CoreError::new(
                    ReasonCode::SpecInvalid,
                    "forgejo fleets must declare a forgejo section",
                ));
            };
            forgejo.validate()?;
        }
        FleetProviderKind::Gitlab => {
            if !is_placeholder_github(&spec.github) || spec.forgejo.is_some() {
                return Err(CoreError::new(
                    ReasonCode::SpecInvalid,
                    "gitlab fleets must not declare a github or forgejo section",
                ));
            }
            let Some(gitlab) = &spec.gitlab else {
                return Err(CoreError::new(
                    ReasonCode::SpecInvalid,
                    "gitlab fleets must declare a gitlab section",
                ));
            };
            gitlab.validate()?;
        }
    }
    spec.capacity
        .validate()
        .map_err(|e| CoreError::new(ReasonCode::SpecInvalid, e))?;
    Ok(())
}

fn validate_labels(labels: &[String]) -> CoreResult<()> {
    if labels.len() > 32 {
        return Err(CoreError::new(ReasonCode::SpecInvalid, "at most 32 labels"));
    }
    for label in labels {
        let label = label.trim();
        if label.is_empty() || label.len() > 64 {
            return Err(CoreError::new(
                ReasonCode::SpecInvalid,
                "each label must be 1..=64 characters",
            ));
        }
    }
    Ok(())
}
