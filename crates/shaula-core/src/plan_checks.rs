//! Check results need operation context: Terraform 1.9.8 leaves a deleted
//! resource's lifecycle conditions unknown because it does not evaluate them.

use std::collections::HashSet;

use serde_json::Value;

use crate::error::CoreResult;
use crate::template::ManagedResourceRole;

use super::{admit_destroy_plan, err, parse_plan_shape, PlannedInstance};

/// Parses and admits a Destroy plan against the exact remaining bound state.
/// Only after ordinary delete admission may an owned resource's skipped
/// lifecycle condition remain unknown. Create callers retain `parse_plan`'s
/// strict check policy and cannot obtain a lenient intermediate plan.
pub fn admit_destroy_plan_json(
    plan_json: &Value,
    state_addresses: &[String],
    manifest_shape: &[ManagedResourceRole],
) -> CoreResult<()> {
    let plan = parse_plan_shape(plan_json)?;
    admit_destroy_plan(&plan, state_addresses, manifest_shape)?;
    admit_checks(plan_json, &plan.instances)
}

pub(super) fn admit_checks(plan_json: &Value, deleted: &[PlannedInstance]) -> CoreResult<()> {
    let Some(checks) = plan_json.get("checks") else {
        return Ok(());
    };
    let checks = checks
        .as_array()
        .ok_or_else(|| err("plan checks must be an array"))?;
    let mut seen = HashSet::new();
    for check in checks {
        if let Some(address) = check.get("address").and_then(|a| a.get("to_display")) {
            let address = address
                .as_str()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| err("plan check address is malformed"))?;
            if !seen.insert(address) {
                return Err(err("plan contains duplicate check addresses"));
            }
        }
        if !empty_or_absent(check, "problems") {
            return Err(err("plan contains check problems"));
        }
        match check.get("status").and_then(Value::as_str) {
            Some("pass") => admit_passed_instances(check)?,
            Some("unknown") if skipped_deleted_resource(check, plan_json, deleted) => {}
            _ => return Err(err("plan contains failed, errored or unresolved checks")),
        }
    }
    Ok(())
}

fn empty_or_absent(value: &Value, member: &str) -> bool {
    value
        .get(member)
        .is_none_or(|value| value.as_array().is_some_and(Vec::is_empty))
}

fn admit_passed_instances(check: &Value) -> CoreResult<()> {
    let Some(instances) = check.get("instances") else {
        return Ok(());
    };
    let instances = instances
        .as_array()
        .ok_or_else(|| err("plan check instances must be an array"))?;
    if instances.iter().any(|instance| {
        instance.get("status").and_then(Value::as_str) != Some("pass")
            || !empty_or_absent(instance, "problems")
    }) {
        return Err(err(
            "plan contains failed, errored or unresolved check instances",
        ));
    }
    Ok(())
}

fn skipped_deleted_resource(check: &Value, plan_json: &Value, deleted: &[PlannedInstance]) -> bool {
    // Admit only the observed aggregate with no instance results. Unknown
    // standalone checks, data sources, outputs, or future shapes stay closed.
    if !empty_or_absent(check, "instances") {
        return false;
    }
    let Some(address) = check.get("address") else {
        return false;
    };
    if address.get("kind").and_then(Value::as_str) != Some("resource")
        || address.get("mode").and_then(Value::as_str) != Some("managed")
    {
        return false;
    }
    let Some(display) = address.get("to_display").and_then(Value::as_str) else {
        return false;
    };
    let Some(resource) = deleted.iter().find(|instance| {
        instance.address == display && instance.mode == "managed" && instance.actions == ["delete"]
    }) else {
        return false;
    };
    if address.get("type").and_then(Value::as_str) != Some(resource.resource_type.as_str()) {
        return false;
    }
    let Some(name) = address
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
    else {
        return false;
    };
    // Compare named identity with the actual resource_change as well as its
    // full address; a forged name/type cannot ride on another bound address.
    plan_json
        .get("resource_changes")
        .and_then(Value::as_array)
        .is_some_and(|changes| {
            changes.iter().any(|change| {
                change.get("address").and_then(Value::as_str) == Some(display)
                    && change.get("name").and_then(Value::as_str) == Some(name)
            })
        })
}
