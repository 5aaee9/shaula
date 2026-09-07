//! Saved-plan admission policy (spec 0004 section 5).
//!
//! Fail-closed parsing of `terraform show -json` output: only supported
//! format majors, `applyable=true`, `complete=true`, `errored=false`;
//! managed actions must be exactly `["create"]` (Create) or `["delete"]`
//! (Destroy); only data resources may use `["read"]`/`["no-op"]`.
//! Deferred changes, import, deposed, move and replacement are rejected.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{CoreError, CoreResult, ReasonCode};
#[cfg(test)]
use crate::template::ManagedResourceRole;

const SUPPORTED_FORMAT_MAJOR: u64 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanIntent {
    Create,
    Destroy,
}

/// One managed resource instance as observed inside the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedInstance {
    pub module_address: Option<String>,
    pub mode: String,
    pub address: String,
    pub resource_type: String,
    pub actions: Vec<String>,
    pub deposed: bool,
    pub previous_address: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanAdmission {
    pub format_version: u64,
    pub terraform_version: Option<String>,
    pub instances: Vec<PlannedInstance>,
    /// Managed resources carried in the plan's `prior_state` snapshot —
    /// an independent witness of non-empty prior state alongside the
    /// `state list` probe (spec 0004 §5.192).
    pub prior_state_managed: usize,
}

fn err(msg: impl Into<String>) -> CoreError {
    CoreError::new(ReasonCode::TemplatePlanFailed, msg)
}

/// Extracts the subset of the plan JSON the policy reasons about. Fails
/// closed on missing/unknown top-level shape.
pub fn parse_plan(plan_json: &Value) -> CoreResult<PlanAdmission> {
    let obj = plan_json
        .as_object()
        .ok_or_else(|| err("plan document is not a JSON object"))?;

    // Terraform reports "major.minor" (e.g. "1.2"); accept the supported
    // major and fail closed on any other major or malformed value.
    let format_version = obj
        .get("format_version")
        .and_then(Value::as_str)
        .and_then(|s| s.split('.').next()?.parse::<u64>().ok())
        .ok_or_else(|| err("plan format_version missing or malformed"))?;
    if format_version != SUPPORTED_FORMAT_MAJOR {
        return Err(err(format!(
            "unsupported plan format major {format_version}"
        )));
    }

    let applyable = obj
        .get("applyable")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let complete = obj
        .get("complete")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let errored = obj.get("errored").and_then(Value::as_bool).unwrap_or(true);
    if !applyable {
        return Err(err("plan is not applyable"));
    }
    if !complete {
        return Err(err("plan is incomplete"));
    }
    if errored {
        return Err(err("plan has errored"));
    }

    let mut instances = Vec::new();
    let empty = Vec::new();
    let changes = obj
        .get("resource_changes")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    for change in changes {
        let cobj = change
            .as_object()
            .ok_or_else(|| err("resource_change entry is not an object"))?;
        let address = cobj
            .get("address")
            .and_then(Value::as_str)
            .ok_or_else(|| err("resource change without address"))?
            .to_string();
        let mode = cobj
            .get("mode")
            .and_then(Value::as_str)
            .ok_or_else(|| err("resource change without mode"))?
            .to_string();
        let resource_type = cobj
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| err("resource change without type"))?
            .to_string();
        let module_address = cobj
            .get("module_address")
            .and_then(Value::as_str)
            .map(str::to_string);

        let change_obj = cobj
            .get("change")
            .and_then(Value::as_object)
            .ok_or_else(|| err("resource change without change object"))?;
        // Unknown modes never bypass admission: only managed/data exist.
        if mode != "managed" && mode != "data" {
            return Err(err(format!("unknown resource mode {mode:?} at {address}")));
        }
        let actions: Vec<String> = change_obj
            .get("actions")
            .and_then(Value::as_array)
            .ok_or_else(|| err("resource change without actions array"))?
            .iter()
            .map(|a| {
                a.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| err("non-string action"))
            })
            .collect::<Result<_, _>>()?;
        // Terraform reports deposed at the resource_change top level and
        // planned imports via `importing` / action_reason; all three are
        // hard rejects (spec 0004 section 5).
        let deposed = cobj.get("deposed").map(|v| !v.is_null()).unwrap_or(false)
            || change_obj
                .get("deposed")
                .map(|v| !v.is_null())
                .unwrap_or(false);
        // Terraform's real wire form is an OBJECT (`{"id": "..."}`); any
        // non-null presence — object or boolean — is a planned import and
        // hard-rejected (spec 0004 §5.191).
        let importing = cobj.get("importing").map(|v| !v.is_null()).unwrap_or(false)
            || change_obj
                .get("importing")
                .map(|v| !v.is_null())
                .unwrap_or(false);
        if importing {
            return Err(err(format!("planned import is rejected: {address}")));
        }
        let action_reason = change_obj
            .get("action_reason")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if action_reason == "import" {
            return Err(err(format!(
                "planned import (action_reason) is rejected: {address}"
            )));
        }
        let previous_address = cobj
            .get("previous_address")
            .and_then(Value::as_str)
            .map(str::to_string);

        instances.push(PlannedInstance {
            module_address,
            mode,
            address,
            resource_type,
            actions,
            deposed,
            previous_address,
        });
    }

    // Deferred changes are rejected outright.
    if obj
        .get("deferred_changes")
        .and_then(Value::as_array)
        .map(|a| !a.is_empty())
        .unwrap_or(false)
    {
        return Err(err("plan contains deferred changes"));
    }

    // Failed/errored standalone checks reject admission when present.
    if let Some(checks) = obj.get("checks").and_then(Value::as_array) {
        for check in checks {
            let problem_count = check
                .get("problems")
                .and_then(Value::as_array)
                .map(|p| p.len())
                .unwrap_or(0);
            if check.get("status").and_then(Value::as_str) != Some("pass") || problem_count > 0 {
                return Err(err("plan contains failed or errored checks"));
            }
        }
    }

    // Managed entries inside the plan's own prior_state snapshot: an
    // independent non-empty-state witness (spec 0004 §5.192). Modules are
    // traversed recursively.
    fn count_managed_resources(module: &Value) -> usize {
        let mut count = module
            .get("resources")
            .and_then(Value::as_array)
            .map(|resources| {
                resources
                    .iter()
                    .filter(|r| r.get("mode").and_then(Value::as_str) == Some("managed"))
                    .count()
            })
            .unwrap_or(0);
        if let Some(children) = module.get("child_modules").and_then(Value::as_array) {
            for child in children {
                count += count_managed_resources(child);
            }
        }
        count
    }
    let prior_state_managed = obj
        .get("prior_state")
        .and_then(|p| p.get("values"))
        .and_then(|v| v.get("root_module"))
        .map(count_managed_resources)
        .unwrap_or(0);

    Ok(PlanAdmission {
        format_version,
        terraform_version: obj
            .get("terraform_version")
            .and_then(Value::as_str)
            .map(str::to_string),
        instances,
        prior_state_managed,
    })
}

#[path = "plan_admit.rs"]
mod plan_admit;
pub use plan_admit::{admit_create_plan, admit_destroy_plan, destroy_empty_state_short_circuit};

#[cfg(test)]
#[path = "plan_tests.rs"]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

#[cfg(test)]
#[path = "plan_admit_tests.rs"]
mod admit_tests;
