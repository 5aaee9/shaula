//! Admission validator tests: the bounded input policy, the schema
//! allowlist grammar and the dialect/enum shape guards (R5-04).
#![allow(clippy::unwrap_used, clippy::panic)]

use super::validate_inputs;
use serde_json::json;

fn inputs(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    value.as_object().unwrap().clone()
}

fn ok(inputs_value: serde_json::Value, policy: &str, schema: Option<&str>) -> bool {
    validate_inputs(&inputs(inputs_value), policy, schema).is_ok()
}

const SCHEMA: &str = r#"{
    "type": "object",
    "additionalProperties": false,
    "required": ["runner_image"],
    "properties": {
        "runner_image": {"type": "string", "enum": ["img:1", "img:2"]},
        "cpu": {"type": "string", "enum": ["1", "2"]}
    }
}"#;

const POLICY: &str = r#"{
    "runner_image": ["img:1", "img:2"],
    "cpu": ["1", "2"]
}"#;

#[test]
fn valid_input_passes_both_layers() {
    assert!(ok(
        json!({"runner_image": "img:1", "cpu": "2"}),
        POLICY,
        Some(SCHEMA)
    ));
}

#[test]
fn missing_required_key_is_rejected() {
    assert!(!ok(json!({"cpu": "1"}), POLICY, Some(SCHEMA)));
}

#[test]
fn wrong_type_is_rejected() {
    let typed = r#"{"type": "object", "properties": {"cpu": {"type": "integer"}}}"#;
    assert!(!ok(json!({"cpu": "1"}), r#"{"cpu": ["1"]}"#, Some(typed)));
    assert!(ok(json!({"cpu": 1}), r#"{"cpu": [1]}"#, Some(typed)));
}

#[test]
fn outside_enum_is_rejected() {
    assert!(!ok(
        json!({"runner_image": "img:3", "cpu": "1"}),
        r#"{"runner_image": ["img:3"], "cpu": ["1"]}"#,
        Some(SCHEMA)
    ));
}

#[test]
fn undeclared_key_is_rejected_by_additional_properties() {
    assert!(!ok(
        json!({"runner_image": "img:1", "gpu": true}),
        r#"{"runner_image": ["img:1"], "gpu": [true]}"#,
        Some(SCHEMA)
    ));
}

#[test]
fn nested_required_and_enum_are_enforced() {
    let nested = r#"{
        "type": "object",
        "properties": {
            "node": {
                "type": "object",
                "additionalProperties": false,
                "required": ["name"],
                "properties": {"name": {"type": "string", "enum": ["a", "b"]}}
            }
        }
    }"#;
    let policy = r#"{"node": [{"name": "a"}, {"name": "b"}]}"#;
    assert!(ok(json!({"node": {"name": "a"}}), policy, Some(nested)));
    // A nested `required: [name]` with `{}` must reject: an unenforced
    // nested constraint is a hole, not a feature.
    assert!(!ok(json!({"node": {}}), policy, Some(nested)));
    // Nested enum applies at the nested level too.
    assert!(!ok(json!({"node": {"name": "z"}}), policy, Some(nested)));
}

#[test]
fn empty_inputs_still_hit_the_required_check() {
    assert!(!ok(json!({}), POLICY, Some(SCHEMA)));
}

#[test]
fn policy_layer_rejects_unknown_keys_and_aliases() {
    // Key admissible by the schema but absent from the finite alias
    // policy is rejected.
    assert!(!ok(
        json!({"runner_image": "img:1"}),
        r#"{"cpu": ["1"]}"#,
        Some(SCHEMA)
    ));
    // Value outside the policy alias is rejected even when the schema
    // enum would allow it.
    assert!(!ok(
        json!({"runner_image": "img:2"}),
        r#"{"runner_image": ["img:1"]}"#,
        Some(SCHEMA)
    ));
}

#[test]
fn too_many_inputs_is_rejected() {
    let mut map = serde_json::Map::new();
    for i in 0..33 {
        map.insert(format!("k{i}"), json!(i));
    }
    let policy = map
        .keys()
        .map(|k| format!(r#""{k}": [0]"#))
        .collect::<Vec<_>>()
        .join(",");
    assert!(validate_inputs(&map, &format!("{{{policy}}}"), None).is_err());
}

#[test]
fn schema_allowlist_rejects_unsupported_constraints() {
    // A constraint the validator cannot enforce must fail admission
    // instead of silently degrading to "unconstrained".
    let with_minimum =
        r#"{"type": "object", "properties": {"cpu": {"type": "integer", "minimum": 1}}}"#;
    assert!(!ok(json!({}), r#"{}"#, Some(with_minimum)));
    // Boolean schemas are unsupported.
    assert!(!ok(json!({}), r#"{}"#, Some("true")));
    // Unsupported type name.
    let null_typed = r#"{"type": "object", "properties": {"cpu": {"type": "null"}}}"#;
    assert!(!ok(json!({}), r#"{}"#, Some(null_typed)));
}

#[test]
fn malformed_schema_shapes_are_rejected() {
    // enum must be an array (R5-04: a non-array enum constrained
    // nothing at validation time).
    assert!(!ok(json!({}), r#"{}"#, Some(r#"{"enum": 42}"#)));
    // required entries must be strings.
    assert!(!ok(json!({}), r#"{}"#, Some(r#"{"required": ["a", 42]}"#)));
    // additionalProperties must be a boolean.
    assert!(!ok(
        json!({}),
        r#"{}"#,
        Some(r#"{"additionalProperties": "no"}"#)
    ));
    // properties must be an object.
    assert!(!ok(json!({}), r#"{}"#, Some(r#"{"properties": []}"#)));
}

#[test]
fn unsupported_dialect_is_rejected_but_standard_ones_pass() {
    assert!(!ok(
        json!({}),
        r#"{}"#,
        Some(r#"{"$schema": "https://example.com/evil-draft"}"#)
    ));
    assert!(ok(
        json!({}),
        r#"{}"#,
        Some(r#"{"$schema": "https://json-schema.org/draft/2020-12/schema"}"#)
    ));
    assert!(ok(
        json!({}),
        r#"{}"#,
        Some(r#"{"$schema": "http://json-schema.org/draft-07/schema#"}"#)
    ));
}

#[test]
fn property_names_are_data_not_keywords() {
    // An input literally named "maximum" validates normally: property
    // NAMES are data, never schema keywords.
    let schema = r#"{
        "type": "object",
        "properties": {"maximum": {"type": "string", "enum": ["ok"]}}
    }"#;
    assert!(ok(
        json!({"maximum": "ok"}),
        r#"{"maximum": ["ok"]}"#,
        Some(schema)
    ));
}

#[test]
fn bundled_profile_schemas_are_admissible() {
    // BOTH bundled profiles ship a $schema dialect declaration and use
    // only the supported grammar: admission must accept them as-is
    // (R5-04). Each validates a real input drawn from its own enum.
    for (path, key, value) in [
        (
            "../../templates/kubernetes/schemas/parameters.schema.json",
            "cpu_request",
            "2",
        ),
        (
            "../../templates/docker/schemas/parameters.schema.json",
            "runner_image",
            "ghcr.io/actions/actions-runner:2.337.0",
        ),
    ] {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let schema_path = std::path::Path::new(manifest_dir).join(path);
        let schema = std::fs::read_to_string(&schema_path)
            .unwrap_or_else(|e| panic!("bundled schema {} must exist: {e}", schema_path.display()));
        let policy = format!(r#"{{"{key}": ["{value}"]}}"#);
        assert!(
            ok(json!({key: value}), &policy, Some(schema.as_str())),
            "bundled schema {} must pass admission",
            schema_path.display()
        );
    }

    // The AWS VM profile admits an empty Fleet input object: no parameters
    // are exposed and additional properties stay rejected.
    let schema_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../templates/aws/schemas/parameters.schema.json");
    let schema = std::fs::read_to_string(&schema_path)
        .unwrap_or_else(|e| panic!("bundled schema {} must exist: {e}", schema_path.display()));
    assert!(
        validate_inputs(&serde_json::Map::new(), "{}", Some(schema.as_str())).is_ok(),
        "bundled schema {} must admit empty Fleet inputs",
        schema_path.display()
    );
    assert!(
        validate_inputs(&inputs(json!({"extra": true})), "{}", Some(schema.as_str())).is_err(),
        "bundled schema {} must reject undeclared inputs",
        schema_path.display()
    );
}

#[test]
fn bundled_proxmox_parameters_admit_cpu_and_memory() {
    let schema_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../templates/proxmox/schemas/parameters.schema.json");
    let schema = std::fs::read_to_string(&schema_path)
        .unwrap_or_else(|e| panic!("bundled schema {} must exist: {e}", schema_path.display()));

    let policy = r#"{
        "cpu_cores": [1, 2, 4, 8],
        "memory_mb": [2048, 4096, 8192, 16384]
    }"#;
    assert!(
        ok(
            json!({"cpu_cores": 4, "memory_mb": 8192}),
            policy,
            Some(schema.as_str())
        ),
        "Proxmox CPU and memory parameters should pass schema and policy admission"
    );

    // Both parameters are integers; quoted values must not pass even if the
    // textual representation appears in the policy.
    assert!(!ok(
        json!({"cpu_cores": "4", "memory_mb": 8192}),
        policy,
        Some(schema.as_str())
    ));
    assert!(!ok(
        json!({"cpu_cores": 16, "memory_mb": 8192}),
        policy,
        Some(schema.as_str())
    ));
    assert!(!ok(
        json!({"cpu_cores": 4, "memory_mb": 32768}),
        policy,
        Some(schema.as_str())
    ));
    assert!(!ok(
        json!({"cpu_cores": 4, "memory_mb": 8192, "extra": true}),
        policy,
        Some(schema.as_str())
    ));
}
