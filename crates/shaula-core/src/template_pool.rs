//! Weighted template pools. A pool selects one immutable template route for
//! each admitted generation; selection policy is implemented by the daemon.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{CoreError, CoreResult, ReasonCode};
use crate::fleet::TemplateProfileRefDto;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PoolFailurePolicy {
    #[default]
    Backpressure,
    Redistribute,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplatePoolMember {
    pub key: String,
    pub template_profile_ref: TemplateProfileRefDto,
    pub weight: u32,
    #[serde(default)]
    pub template_inputs: Map<String, Value>,
    #[serde(default)]
    pub max_runners: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplatePoolSpec {
    pub members: Vec<TemplatePoolMember>,
    #[serde(default)]
    pub failure_policy: PoolFailurePolicy,
}

impl TemplatePoolSpec {
    pub fn validate(&self) -> CoreResult<()> {
        if self.members.is_empty() || self.members.len() > 32 {
            return Err(CoreError::new(
                ReasonCode::SpecInvalid,
                "template pool must contain 1..=32 members",
            ));
        }
        let mut keys = std::collections::HashSet::new();
        for member in &self.members {
            if !crate::auth::is_stable_identifier(&member.key)
                || member.key.len() > 100
                || !keys.insert(&member.key)
            {
                return Err(CoreError::new(
                    ReasonCode::SpecInvalid,
                    "template pool member keys must be unique stable identifiers",
                ));
            }
            if member.weight == 0 || member.weight > 10_000 {
                return Err(CoreError::new(
                    ReasonCode::SpecInvalid,
                    "template pool member weight must be 1..=10000",
                ));
            }
            if member.max_runners.is_some_and(|value| value < 0) {
                return Err(CoreError::new(
                    ReasonCode::SpecInvalid,
                    "template pool member max_runners must be non-negative",
                ));
            }
            if member.template_profile_ref.is_empty() {
                return Err(CoreError::new(
                    ReasonCode::SpecInvalid,
                    "template pool member template_profile_ref is required",
                ));
            }
        }
        Ok(())
    }
}

/// Maps uniform entropy to a member interval without modulo bias. `None`
/// means the entropy sample fell in the rejected tail and the caller should
/// draw a fresh sample.
pub fn weighted_member_index(weights: &[u32], entropy: u128) -> Option<usize> {
    let total: u128 = weights.iter().map(|weight| u128::from(*weight)).sum();
    if total == 0 {
        return None;
    }
    let limit = u128::MAX - (u128::MAX % total);
    if entropy >= limit {
        return None;
    }
    let draw = entropy % total;
    let mut offset = 0_u128;
    weights.iter().position(|weight| {
        offset += u128::from(*weight);
        draw < offset
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedTemplatePoolMember {
    pub key: String,
    pub template_profile_key: String,
    pub template_revision: i64,
    pub template_artifact_digest: String,
    pub template_attestation_id: String,
    pub template_inputs: Map<String, Value>,
    pub inputs_digest: String,
    pub weight: u32,
    pub max_runners: Option<i64>,
}

/// Result returned by pool admission before the scheduler persists it.
#[derive(Debug, Clone)]
pub struct PoolGenerationAdmission {
    pub generation: crate::registry::GenerationRecord,
    pub template_inputs: Map<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::weighted_member_index;

    #[test]
    fn weighted_intervals_are_independent_of_history() {
        let weights = [10, 20];
        assert_eq!(weighted_member_index(&weights, 0), Some(0));
        assert_eq!(weighted_member_index(&weights, 9), Some(0));
        assert_eq!(weighted_member_index(&weights, 10), Some(1));
        assert_eq!(weighted_member_index(&weights, 29), Some(1));
    }

    #[test]
    fn zero_weights_have_no_selection() {
        assert_eq!(weighted_member_index(&[0, 0], 0), None);
    }
}

/// Desired-state head of one shared TemplatePool resource (spec 0037).
#[derive(Debug, Clone)]
pub struct TemplatePoolHead {
    pub key: String,
    pub incarnation: String,
    pub desired_revision: i64,
    pub phase: String,
    pub deletion_marker: bool,
    pub tombstone: bool,
}

/// One immutable shared-pool revision with its resolved members (spec
/// 0037 §3): member `template_profile_ref` bare keys are resolved to the
/// profiles' then-current Active revisions and frozen on these rows.
#[derive(Debug, Clone)]
pub struct TemplatePoolRevision {
    pub pool_key: String,
    pub revision: i64,
    pub spec_json: String,
    pub failure_policy: PoolFailurePolicy,
    pub members: Vec<ResolvedTemplatePoolMember>,
    pub actor: Option<String>,
    pub created_at: i64,
}

/// The pool reference a fleet revision freezes at admission: the pool key
/// plus the exact pool revision whose member rows route its generations.
pub type FleetPoolRef = (String, i64);
