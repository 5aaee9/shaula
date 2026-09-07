// Unit tests extracted to their own module to keep the host file
// within the 400-line limit (AGENTS.md).
use super::*;

pub(super) fn shape() -> Vec<ManagedResourceRole> {
    vec![
        ManagedResourceRole {
            role: "bootstrap".into(),
            terraform_type: "kubernetes_secret_v1".into(),
            exact_count: 1,
        },
        ManagedResourceRole {
            role: "runner".into(),
            terraform_type: "kubernetes_pod_v1".into(),
            exact_count: 1,
        },
    ]
}

pub(super) fn plan_json(resource_changes: Value) -> Value {
    serde_json::json!({
        "format_version": "1.2",
        "terraform_version": "1.9.0",
        "applyable": true,
        "complete": true,
        "errored": false,
        "resource_changes": resource_changes
    })
}

pub(super) fn change(mode: &str, r#type: &str, address: &str, actions: &[&str]) -> Value {
    serde_json::json!({
        "address": address,
        "mode": mode,
        "type": r#type,
        "name": "x",
        "change": {"actions": actions}
    })
}

#[test]
fn valid_create_plan_admits() {
    let json = plan_json(serde_json::json!([
        change(
            "managed",
            "kubernetes_secret_v1",
            "kubernetes_secret_v1.bootstrap",
            &["create"]
        ),
        change(
            "managed",
            "kubernetes_pod_v1",
            "kubernetes_pod_v1.runner",
            &["create"]
        ),
        change(
            "data",
            "kubernetes_namespace",
            "data.kubernetes_namespace.ns",
            &["read"]
        ),
    ]));
    let parsed = parse_plan(&json).unwrap();
    assert!(admit_create_plan(&parsed, true, &shape()).is_ok());
}

#[test]
fn unsupported_format_major_rejected() {
    let mut json = plan_json(serde_json::json!([]));
    json["format_version"] = serde_json::json!("2.0");
    assert!(parse_plan(&json).is_err());
}

#[test]
fn non_applyable_or_errored_rejected() {
    for (key, value) in [("applyable", false), ("complete", false), ("errored", true)] {
        let mut json = plan_json(serde_json::json!([]));
        json[key] = serde_json::json!(value);
        assert!(parse_plan(&json).is_err(), "{key}={value} must reject");
    }
}

#[test]
fn create_rejects_non_empty_prior_state() {
    let json = plan_json(serde_json::json!([
        change(
            "managed",
            "kubernetes_secret_v1",
            "kubernetes_secret_v1.bootstrap",
            &["create"]
        ),
        change(
            "managed",
            "kubernetes_pod_v1",
            "kubernetes_pod_v1.runner",
            &["create"]
        ),
    ]));
    let parsed = parse_plan(&json).unwrap();
    assert!(admit_create_plan(&parsed, false, &shape()).is_err());
}

#[test]
fn create_rejects_replacement_and_update() {
    for actions in [
        ["delete", "create"],
        ["create", "delete"],
        ["update", "update"],
        ["delete", "delete"],
    ] {
        let json = plan_json(serde_json::json!([
            change(
                "managed",
                "kubernetes_secret_v1",
                "kubernetes_secret_v1.bootstrap",
                &["create"]
            ),
            change(
                "managed",
                "kubernetes_pod_v1",
                "kubernetes_pod_v1.runner",
                &actions
            ),
        ]));
        let parsed = parse_plan(&json).unwrap();
        assert!(
            admit_create_plan(&parsed, true, &shape()).is_err(),
            "{actions:?} must reject"
        );
    }
}

#[test]
fn create_rejects_undeclared_type_and_wrong_cardinality() {
    let undeclared = plan_json(serde_json::json!([
        change("managed", "kubernetes_secret_v1", "s", &["create"]),
        change("managed", "kubernetes_pod_v1", "p", &["create"]),
        change("managed", "kubernetes_service_v1", "svc", &["create"]),
    ]));
    let parsed = parse_plan(&undeclared).unwrap();
    assert!(admit_create_plan(&parsed, true, &shape()).is_err());

    let double_pod = plan_json(serde_json::json!([
        change("managed", "kubernetes_secret_v1", "s", &["create"]),
        change("managed", "kubernetes_pod_v1", "p1", &["create"]),
        change("managed", "kubernetes_pod_v1", "p2", &["create"]),
    ]));
    let parsed = parse_plan(&double_pod).unwrap();
    assert!(admit_create_plan(&parsed, true, &shape()).is_err());
}

#[test]
fn data_resource_update_rejected() {
    let json = plan_json(serde_json::json!([
        change("managed", "kubernetes_secret_v1", "s", &["create"]),
        change("managed", "kubernetes_pod_v1", "p", &["create"]),
        change("data", "kubernetes_namespace", "data.ns", &["update"]),
    ]));
    let parsed = parse_plan(&json).unwrap();
    assert!(admit_create_plan(&parsed, true, &shape()).is_err());
}

#[test]
fn deferred_changes_rejected() {
    let mut json = plan_json(serde_json::json!([]));
    json["deferred_changes"] = serde_json::json!([{"foo": 1}]);
    assert!(parse_plan(&json).is_err());
}

#[test]
fn failed_checks_rejected_but_advisory_only_note() {
    let mut json = plan_json(serde_json::json!([]));
    json["checks"] = serde_json::json!([{"status": "fail", "problems": [{"message": "x"}]}]);
    assert!(parse_plan(&json).is_err());
}

#[test]
fn create_rejects_nonempty_prior_state_snapshot() {
    let json = plan_json(serde_json::json!([
        change(
            "managed",
            "kubernetes_secret_v1",
            "kubernetes_secret_v1.bootstrap",
            &["create"]
        ),
        change(
            "managed",
            "kubernetes_pod_v1",
            "kubernetes_pod_v1.runner",
            &["create"]
        ),
    ]));
    let with_prior_state = serde_json::json!({
        "format_version": "1.2",
        "terraform_version": "1.9.0",
        "applyable": true,
        "complete": true,
        "errored": false,
        "resource_changes": json["resource_changes"],
        "prior_state": {"values": {"root_module": {"resources": [
            {"mode": "managed", "type": "kubernetes_secret_v1", "name": "old",
             "address": "kubernetes_secret_v1.old",
             "values": {}}
        ]}}},
    });
    let parsed = parse_plan(&with_prior_state).unwrap();
    assert_eq!(parsed.prior_state_managed, 1);
    assert!(admit_create_plan(&parsed, true, &shape()).is_err());
}
