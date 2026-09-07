//! Saved-plan admission functions, split to keep files within 400 lines
//! (AGENTS.md). Policy: managed actions must be exactly `["create"]`
//! (Create) or `["delete"]` (Destroy); only data resources may use
//! `["read"]`/`["no-op"]`. Deferred changes, import, deposed, move and
//! replacement are rejected.

use crate::error::CoreResult;
use crate::template::ManagedResourceRole;

use super::err;
use super::PlanAdmission;

/// empty managed prior state, every declared managed instance exactly once
/// with actions exactly `["create"]`; data resources may only read/no-op.
pub fn admit_create_plan(
    plan: &PlanAdmission,
    prior_state_empty: bool,
    manifest_shape: &[ManagedResourceRole],
) -> CoreResult<()> {
    if !prior_state_empty {
        return Err(err("create requires empty managed prior state"));
    }
    if plan.prior_state_managed > 0 {
        return Err(err(
            "create requires empty managed prior state (plan prior_state snapshot is not empty)",
        ));
    }
    admit_managed(plan, manifest_shape, &["create".to_string()], "create")?;
    admit_data_resources(plan)
}

/// Admits a parsed plan for Destroy. An already-empty bound state succeeds
/// without apply (handled by the caller); otherwise every managed instance
/// must be exactly `["delete"]` and no new address or type may appear.
pub fn admit_destroy_plan(
    plan: &PlanAdmission,
    state_addresses: &[String],
    manifest_shape: &[ManagedResourceRole],
) -> CoreResult<()> {
    // Destroy uses the bound state’s remaining instances as the basis:
    // after a partial destroy the plan may cover a SUBSET of the original
    // collection, so full manifest cardinality does not apply here.
    // Each planned managed instance must still be a declared type with
    // exact ["delete"] actions, and every planned address must exist in
    // the bound state (no new addresses, spec 0004 §5.5).
    let declared_types: Vec<&str> = manifest_shape
        .iter()
        .map(|r| r.terraform_type.as_str())
        .collect();
    for instance in plan.instances.iter().filter(|i| i.mode == "managed") {
        if instance.deposed {
            return Err(err(format!(
                "deposed instance in delete plan: {}",
                instance.address
            )));
        }
        if instance.previous_address.is_some() {
            return Err(err(format!(
                "moved resource in delete plan: {}",
                instance.address
            )));
        }
        if instance.actions.len() != 1 || instance.actions[0] != "delete" {
            return Err(err(format!(
                "delete plan requires actions exactly [delete] but found {:?} at {}",
                instance.actions, instance.address
            )));
        }
        if !declared_types.contains(&instance.resource_type.as_str()) {
            return Err(err(format!(
                "undeclared managed resource type in delete plan: {}",
                instance.resource_type
            )));
        }
    }
    admit_data_resources(plan)?;

    // The planned managed address set must correspond EXACTLY to the bound
    // state (spec 0004 §5.192–193): after a partial destroy the bound
    // state itself shrank, so the plan covers every REMAINING instance —
    // no new addresses, no missing orphans, no duplicates.
    let mut planned: Vec<&str> = plan
        .instances
        .iter()
        .filter(|i| i.mode == "managed")
        .map(|i| i.address.as_str())
        .collect();
    planned.sort_unstable();
    if planned.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(err("destroy plan lists a managed address more than once"));
    }
    for address in &planned {
        if !state_addresses.iter().any(|s| s == address) {
            return Err(err(
                "destroy plan introduces address not present in state: address out of bound state"
                    .to_string(),
            ));
        }
    }
    for state_address in state_addresses {
        if !planned.contains(&state_address.as_str()) {
            return Err(err(format!(
                "destroy plan omits bound-state address {state_address:?}: the apply would orphan it"
            )));
        }
    }
    Ok(())
}

fn admit_managed(
    plan: &PlanAdmission,
    manifest_shape: &[ManagedResourceRole],
    exact_actions: &[String],
    kind: &str,
) -> CoreResult<()> {
    let mut per_role: Vec<(&ManagedResourceRole, usize)> =
        manifest_shape.iter().map(|r| (r, 0usize)).collect();
    for instance in &plan.instances {
        if instance.mode != "managed" {
            continue;
        }
        // Reject imports/deposed/moves/replacements unconditionally.
        if instance.deposed {
            return Err(err(format!(
                "deposed instance in {kind} plan: {}",
                instance.address
            )));
        }
        if instance.previous_address.is_some() {
            return Err(err(format!(
                "moved resource in {kind} plan: {}",
                instance.address
            )));
        }
        if instance
            .actions
            .iter()
            .any(|a| a == "import" || a == "move" || a == "forget")
        {
            return Err(err(format!(
                "forbidden action in {kind} plan: {}",
                instance.address
            )));
        }
        // Replacement orderings ("create","delete") and ("delete","create")
        // are both rejected - only the exact single action is allowed.
        if instance.actions.len() != 1 || instance.actions[0] != exact_actions[0] {
            return Err(err(format!(
                "{kind} plan requires actions exactly [{:?}] but found {:?} at {}",
                exact_actions[0], instance.actions, instance.address
            )));
        }
        // The resource type must be declared by the manifest.
        let matched = per_role
            .iter_mut()
            .find(|(role, _)| role.terraform_type == instance.resource_type);
        let Some((_, count)) = matched else {
            return Err(err(format!(
                "undeclared managed resource type in {kind} plan: {}",
                instance.resource_type
            )));
        };
        *count += 1;
    }
    for (role, count) in per_role {
        if count != role.exact_count as usize {
            return Err(err(format!(
                "{kind} plan role '{}' expects exactly {} instance(s) of '{}', found {count}",
                role.role, role.exact_count, role.terraform_type
            )));
        }
    }
    Ok(())
}

fn admit_data_resources(plan: &PlanAdmission) -> CoreResult<()> {
    for instance in &plan.instances {
        if instance.mode != "data" {
            continue;
        }
        let allowed = instance.actions.len() == 1
            && (instance.actions[0] == "read" || instance.actions[0] == "no-op");
        if !allowed {
            return Err(err(format!(
                "data resource may only read or no-op, found {:?} at {}",
                instance.actions, instance.address
            )));
        }
    }
    Ok(())
}

/// Whether an already-empty bound state short-circuits destroy (success
/// without apply).
pub fn destroy_empty_state_short_circuit(state_addresses: &[String]) -> bool {
    state_addresses.is_empty()
}
