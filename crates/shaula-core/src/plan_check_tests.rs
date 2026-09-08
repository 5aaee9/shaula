//! Boundaries around Terraform's skipped resource conditions during Destroy.

use serde_json::{json, Value};

use super::{admit_destroy_plan_json, parse_plan};
use crate::template::ManagedResourceRole;

fn destroy_plan() -> Value {
    json!({
        "format_version": "1.2", "terraform_version": "1.9.8",
        "applyable": true, "complete": true, "errored": false,
        "resource_changes": [{
            "address": "docker_container.runner", "mode": "managed",
            "type": "docker_container", "name": "runner",
            "change": {"actions": ["delete"]}
        }],
        "checks": [{
            "address": {
                "kind": "resource", "mode": "managed", "name": "runner",
                "to_display": "docker_container.runner", "type": "docker_container"
            },
            "status": "unknown"
        }]
    })
}

fn admits_destroy(plan: &Value, addresses: &[&str]) -> bool {
    let state: Vec<String> = addresses
        .iter()
        .map(|address| (*address).to_string())
        .collect();
    let shape = [ManagedResourceRole {
        role: "runner".into(),
        terraform_type: "docker_container".into(),
        exact_count: 1,
    }];
    admit_destroy_plan_json(plan, &state, &shape).is_ok()
}

#[test]
fn skipped_destroy_checks_do_not_weaken_create_parsing() {
    let mut plan = destroy_plan();
    assert!(parse_plan(&plan).is_err());
    plan["resource_changes"][0]["change"]["actions"] = json!(["create"]);
    assert!(parse_plan(&plan).is_err());
    assert!(!admits_destroy(&plan, &["docker_container.runner"]));
    plan["checks"][0]["status"] = json!("pass");
    assert!(parse_plan(&plan).is_ok());
}

#[test]
fn skipped_checks_require_exact_bound_deleted_identity() {
    let original = destroy_plan();
    assert!(!admits_destroy(&original, &[]));
    assert!(!admits_destroy(&original, &["docker_container.other"]));
    for (member, value) in [
        ("kind", json!("check")),
        ("kind", json!("output_value")),
        ("mode", json!("data")),
        ("name", json!("stranger")),
        ("name", json!("")),
        ("type", json!("docker_network")),
        ("to_display", json!("docker_container.other")),
        ("to_display", json!("")),
        ("to_display", Value::Null),
    ] {
        let mut plan = original.clone();
        plan["checks"][0]["address"][member] = value;
        assert!(
            !admits_destroy(&plan, &["docker_container.runner"]),
            "{member}"
        );
    }
    let mut missing_address = original;
    missing_address["checks"][0] = json!({"status": "unknown"});
    assert!(!admits_destroy(
        &missing_address,
        &["docker_container.runner"]
    ));
}

#[test]
fn skipped_checks_accept_empty_results_but_not_missing_identity_members() {
    let mut plan = destroy_plan();
    plan["checks"][0]["problems"] = json!([]);
    plan["checks"][0]["instances"] = json!([]);
    assert!(admits_destroy(&plan, &["docker_container.runner"]));
    for member in ["kind", "mode", "name", "type", "to_display"] {
        let mut missing = plan.clone();
        if let Some(address) = missing["checks"][0]["address"].as_object_mut() {
            address.remove(member);
        }
        assert!(
            !admits_destroy(&missing, &["docker_container.runner"]),
            "{member}"
        );
    }
}

#[test]
fn skipped_checks_never_accept_failures_problems_or_ambiguous_shapes() {
    for status in ["fail", "error", "pending", "", "PASS"] {
        let mut plan = destroy_plan();
        plan["checks"][0]["status"] = json!(status);
        assert!(!admits_destroy(&plan, &["docker_container.runner"]));
    }
    for problems in [json!([{"message": "failed"}]), json!({}), Value::Null] {
        let mut plan = destroy_plan();
        plan["checks"][0]["problems"] = problems;
        assert!(!admits_destroy(&plan, &["docker_container.runner"]));
    }
    let mut duplicate = destroy_plan();
    duplicate["checks"] = json!([duplicate["checks"][0], duplicate["checks"][0]]);
    assert!(!admits_destroy(&duplicate, &["docker_container.runner"]));
    for checks in [json!({}), Value::Null, json!([{}])] {
        let mut plan = destroy_plan();
        plan["checks"] = checks;
        assert!(!admits_destroy(&plan, &["docker_container.runner"]));
    }
}

#[test]
fn skipped_check_exception_preserves_delete_action_and_shape_policy() {
    for actions in [
        json!(["no-op"]),
        json!(["update"]),
        json!(["delete", "create"]),
    ] {
        let mut plan = destroy_plan();
        plan["resource_changes"][0]["change"]["actions"] = actions;
        assert!(!admits_destroy(&plan, &["docker_container.runner"]));
    }
    for member in ["importing", "deposed", "previous_address"] {
        let mut plan = destroy_plan();
        plan["resource_changes"][0][member] = json!("not-admitted");
        assert!(!admits_destroy(&plan, &["docker_container.runner"]));
    }
    let mut duplicate = destroy_plan();
    duplicate["resource_changes"] = json!([
        duplicate["resource_changes"][0],
        duplicate["resource_changes"][0]
    ]);
    assert!(!admits_destroy(&duplicate, &["docker_container.runner"]));
}

#[test]
fn passed_aggregate_cannot_hide_failed_instance_results() {
    for status in ["fail", "error", "unknown"] {
        let mut plan = destroy_plan();
        plan["checks"][0]["status"] = json!("pass");
        plan["checks"][0]["instances"] = json!([{"status": status}]);
        assert!(!admits_destroy(&plan, &["docker_container.runner"]));
        assert!(parse_plan(&plan).is_err());
    }
    let mut unknown = destroy_plan();
    unknown["checks"][0]["instances"] = json!([{"status": "pass"}]);
    assert!(!admits_destroy(&unknown, &["docker_container.runner"]));
}
