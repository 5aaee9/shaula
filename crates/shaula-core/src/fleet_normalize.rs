//! Fleet spec validation and normalization into immutable identity parts.

use super::{is_placeholder_github, FleetProviderKind, FleetSpec, NormalizedFleet};
use crate::auth::validate_profile_key_for_fleet_ref;
use crate::capacity::CapacityPolicy;
use crate::error::{CoreError, CoreResult, ReasonCode};
use crate::forgejo::validate_labels as validate_forgejo_labels;
use crate::github::{Label, ScaleSetIdentity};

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
            validate_forgejo_labels(&forgejo.labels)?;
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
    CapacityPolicy::from(spec.capacity)
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

/// Normalizes a validated spec into its immutable identity parts.
pub fn normalize_fleet(spec: &FleetSpec, inputs_digest: String) -> CoreResult<NormalizedFleet> {
    validate_fleet_spec(spec)?;
    let (identity, forgejo_target, gitlab_target, labels) = match spec.kind {
        FleetProviderKind::Github => (
            Some(ScaleSetIdentity {
                target: spec.github.target.clone(),
                runner_group: spec.github.runner_group.trim().to_string(),
                scale_set_name: spec.github.scale_set_name.trim().to_string(),
            }),
            None,
            None,
            spec.github.labels.clone(),
        ),
        FleetProviderKind::Forgejo => {
            let forgejo = spec.forgejo.as_ref().ok_or_else(|| {
                CoreError::new(ReasonCode::SpecInvalid, "forgejo section is required")
            })?;
            (None, Some(forgejo.target()), None, forgejo.labels.clone())
        }
        FleetProviderKind::Gitlab => {
            let gitlab = spec.gitlab.as_ref().ok_or_else(|| {
                CoreError::new(ReasonCode::SpecInvalid, "gitlab section is required")
            })?;
            // Tags are not runner labels; the matching tuple lives in the section.
            (None, None, Some(gitlab.target()), Vec::new())
        }
    };
    Ok(NormalizedFleet {
        provider: spec.kind,
        identity,
        forgejo_target,
        gitlab_target,
        labels: labels
            .iter()
            .map(|label| Label::system(label.trim()))
            .collect::<CoreResult<Vec<_>>>()?,
        capacity: CapacityPolicy::from(spec.capacity),
        inputs_digest,
    })
}
