//! Destroy-side and F13 negative-path admission tests, split to keep
//! files within 400 lines (AGENTS.md).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::tests::{change, plan_json, shape};
use super::*;

#[test]
fn destroy_plan_admits_exact_deletes() {
    let json = plan_json(serde_json::json!([
        change(
            "managed",
            "kubernetes_pod_v1",
            "kubernetes_pod_v1.runner",
            &["delete"]
        ),
        change(
            "managed",
            "kubernetes_secret_v1",
            "kubernetes_secret_v1.bootstrap",
            &["delete"]
        ),
    ]));
    let parsed = parse_plan(&json).unwrap();
    let state = vec![
        "kubernetes_pod_v1.runner".to_string(),
        "kubernetes_secret_v1.bootstrap".to_string(),
    ];
    assert!(admit_destroy_plan(&parsed, &state, &shape()).is_ok());
}

#[test]
fn terraform_1_9_8_destroy_skips_owned_resource_precondition() {
    // Sanitized shape from a real Terraform 1.9.8 Docker destroy plan.
    // Terraform does not evaluate this resource's precondition on deletion.
    let json = serde_json::json!({
        "format_version": "1.2",
        "terraform_version": "1.9.8",
        "applyable": true,
        "complete": true,
        "errored": false,
        "resource_changes": [{
            "address": "docker_container.runner",
            "mode": "managed",
            "type": "docker_container",
            "name": "runner",
            "change": {"actions": ["delete"]}
        }],
        "checks": [{
            "address": {
                "kind": "resource",
                "mode": "managed",
                "name": "runner",
                "to_display": "docker_container.runner",
                "type": "docker_container"
            },
            "status": "unknown"
        }]
    });
    let state = vec!["docker_container.runner".to_string()];
    let manifest = [ManagedResourceRole {
        role: "runner".into(),
        terraform_type: "docker_container".into(),
        exact_count: 1,
    }];
    let admitted = admit_destroy_plan_json(&json, &state, &manifest);
    assert!(
        admitted.is_ok(),
        "a bound delete must admit skipped checks: {admitted:?}"
    );
}

#[test]
fn destroy_rejects_new_address_not_in_state() {
    let json = plan_json(serde_json::json!([
        change(
            "managed",
            "kubernetes_pod_v1",
            "kubernetes_pod_v1.runner",
            &["delete"]
        ),
        change(
            "managed",
            "kubernetes_secret_v1",
            "kubernetes_secret_v1.bootstrap",
            &["delete"]
        ),
        change(
            "managed",
            "kubernetes_secret_v1",
            "kubernetes_secret_v1.stranger",
            &["delete"]
        ),
    ]));
    let parsed = parse_plan(&json).unwrap();
    let state = vec![
        "kubernetes_pod_v1.runner".to_string(),
        "kubernetes_secret_v1.bootstrap".to_string(),
    ];
    assert!(admit_destroy_plan(&parsed, &state, &shape()).is_err());
}

#[test]
fn destroy_rejects_create_action() {
    let json = plan_json(serde_json::json!([
        change(
            "managed",
            "kubernetes_pod_v1",
            "kubernetes_pod_v1.runner",
            &["delete"]
        ),
        change(
            "managed",
            "kubernetes_secret_v1",
            "kubernetes_secret_v1.bootstrap",
            &["create"]
        ),
    ]));
    let parsed = parse_plan(&json).unwrap();
    let state = vec![
        "kubernetes_pod_v1.runner".to_string(),
        "kubernetes_secret_v1.bootstrap".to_string(),
    ];
    assert!(admit_destroy_plan(&parsed, &state, &shape()).is_err());
}

#[test]
fn empty_state_destroy_short_circuits() {
    assert!(destroy_empty_state_short_circuit(&[]));
    assert!(!destroy_empty_state_short_circuit(&["a".into()]));
}

#[test]
fn missing_plan_headers_fail_closed() {
    assert!(parse_plan(&serde_json::json!({})).is_err());
    assert!(parse_plan(&serde_json::json!([])).is_err());
    assert!(parse_plan(&serde_json::json!({"format_version": "1.2"})).is_err());
}

#[test]
fn destroy_subset_after_partial_destroy_admits() {
    // Kubernetes Secret created, Pod create failed; the cleanup plan only
    // deletes the remaining Secret (subset of the original collection).
    let json = plan_json(serde_json::json!([change(
        "managed",
        "kubernetes_secret_v1",
        "kubernetes_secret_v1.bootstrap",
        &["delete"]
    ),]));
    let parsed = parse_plan(&json).unwrap();
    let state = vec!["kubernetes_secret_v1.bootstrap".to_string()];
    assert!(admit_destroy_plan(&parsed, &state, &shape()).is_ok());
}

// ---- F13 negative paths: import / deposed / moved / unknown mode ----

fn with_change_extra(top_level: Value, change_level: Value) -> Value {
    let mut c = change(
        "managed",
        "kubernetes_secret_v1",
        "kubernetes_secret_v1.bootstrap",
        &["delete"],
    );
    for (k, v) in top_level.as_object().expect("object") {
        c[k.as_str()] = v.clone();
    }
    for (k, v) in change_level.as_object().expect("object") {
        c["change"][k.as_str()] = v.clone();
    }
    c
}

#[test]
fn planned_import_flag_is_rejected() {
    // Real Terraform wire form: `importing` is an OBJECT (`{"id": ..}`);
    // boolean and top-level variants must not slip past either.
    for (top_level, change_level) in [
        (
            serde_json::json!({}),
            serde_json::json!({"importing": {"id": "secret/bootstrap"}}),
        ),
        (
            serde_json::json!({}),
            serde_json::json!({"importing": true}),
        ),
        (
            serde_json::json!({"importing": {"id": "x"}}),
            serde_json::json!({}),
        ),
    ] {
        let c = with_change_extra(top_level, change_level);
        let json = plan_json(serde_json::json!([c]));
        assert!(parse_plan(&json).is_err());
    }
}

#[test]
fn import_action_reason_is_rejected() {
    let c = with_change_extra(
        serde_json::json!({}),
        serde_json::json!({"action_reason": "import"}),
    );
    let json = plan_json(serde_json::json!([c]));
    assert!(parse_plan(&json).is_err());
}

#[test]
fn deposed_instance_is_rejected_on_both_paths() {
    for extra in [
        // Terraform reports deposed at the resource_change top level; a
        // nested copy must not slip past either.
        (
            serde_json::json!({"deposed": "bd1abc"}),
            serde_json::json!({}),
        ),
        (serde_json::json!({}), serde_json::json!({"deposed": true})),
    ] {
        let c = with_change_extra(extra.0, extra.1);
        let json = plan_json(serde_json::json!([c]));
        let parsed = parse_plan(&json).unwrap();
        let state = vec!["kubernetes_secret_v1.bootstrap".to_string()];
        assert!(admit_create_plan(&parsed, true, &shape()).is_err());
        assert!(admit_destroy_plan(&parsed, &state, &shape()).is_err());
    }
}

#[test]
fn moved_resource_is_rejected_on_both_paths() {
    let c = with_change_extra(
        serde_json::json!({"previous_address": "kubernetes_secret_v1.old"}),
        serde_json::json!({}),
    );
    let json = plan_json(serde_json::json!([c]));
    let parsed = parse_plan(&json).unwrap();
    let state = vec!["kubernetes_secret_v1.bootstrap".to_string()];
    assert!(admit_create_plan(&parsed, true, &shape()).is_err());
    assert!(admit_destroy_plan(&parsed, &state, &shape()).is_err());
}

#[test]
fn unknown_resource_mode_is_rejected() {
    let c = change(
        "orthogonal",
        "kubernetes_secret_v1",
        "kubernetes_secret_v1.bootstrap",
        &["create"],
    );
    let json = plan_json(serde_json::json!([c]));
    assert!(parse_plan(&json).is_err());
}

#[test]
fn destroy_rejects_duplicate_and_omitted_state_addresses() {
    let instance = || {
        change(
            "managed",
            "kubernetes_secret_v1",
            "kubernetes_secret_v1.bootstrap",
            &["delete"],
        )
    };
    let state = vec!["kubernetes_secret_v1.bootstrap".to_string()];

    // Duplicate planned address: the apply must never delete one address
    // twice.
    let json = plan_json(serde_json::json!([instance(), instance()]));
    let parsed = parse_plan(&json).unwrap();
    assert!(admit_destroy_plan(&parsed, &state, &shape()).is_err());

    // An address present in the bound state but missing from the plan
    // would be orphaned by the apply.
    let full_state = vec![
        "kubernetes_secret_v1.bootstrap".to_string(),
        "kubernetes_pod_v1.runner".to_string(),
    ];
    let json = plan_json(serde_json::json!([instance()]));
    let parsed = parse_plan(&json).unwrap();
    assert!(admit_destroy_plan(&parsed, &full_state, &shape()).is_err());

    // Exact correspondence admits.
    let runner = change(
        "managed",
        "kubernetes_pod_v1",
        "kubernetes_pod_v1.runner",
        &["delete"],
    );
    let json = plan_json(serde_json::json!([instance(), runner]));
    let parsed = parse_plan(&json).unwrap();
    assert!(admit_destroy_plan(&parsed, &full_state, &shape()).is_ok());
}
