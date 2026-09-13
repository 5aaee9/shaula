//! Unit tests for the spec 0038 bindings split (sensitivity, projection,
//! update merge and structural validation).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use serde_json::json;

fn proxmox_like_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["proxmox_host", "proxmox_token"],
        "properties": {
            "proxmox_host": {"type": "string", "sensitive": false},
            "proxmox_token": {"type": "string", "minLength": 1, "sensitive": true},
            "proxmox_insecure": {"type": "boolean", "sensitive": false},
            "proxmox_vmid_begin": {"type": "integer", "minimum": 100, "sensitive": false},
            "unannotated": {"type": "string"},
            "nested_secret_member": {
                "type": "object",
                "sensitive": false,
                "properties": {"token": {"type": "string", "sensitive": true}}
            }
        }
    })
}

fn map(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap()
}

#[test]
fn projection_exposes_non_secret_values_and_presence_markers_only() {
    let schema = BindingsSchema::parse(&proxmox_like_schema()).unwrap();
    let stored = map(json!({
        "proxmox_host": "https://pve.example.com:8006",
        "proxmox_token": "user@pam!id=secret-value",
        "proxmox_insecure": true,
        "proxmox_vmid_begin": 200,
    }));
    let projected = schema.project(&stored);
    assert_eq!(
        projected.get("proxmox_host"),
        Some(&json!("https://pve.example.com:8006"))
    );
    assert_eq!(projected.get("proxmox_insecure"), Some(&json!(true)));
    assert_eq!(projected.get("proxmox_vmid_begin"), Some(&json!(200)));
    // Secret presence marker only — never the value.
    assert_eq!(
        projected.get("proxmox_token"),
        Some(&json!({"sensitive": true, "set": true}))
    );
    let encoded = serde_json::to_string(&projected).unwrap();
    assert!(!encoded.contains("secret-value"));
}

#[test]
fn unknown_and_unannotated_fields_fail_closed_to_markers() {
    let schema = BindingsSchema::parse(&proxmox_like_schema()).unwrap();
    let stored = map(json!({
        "unannotated": "some value",
        "not_in_schema": "another value"
    }));
    let projected = schema.project(&stored);
    assert_eq!(
        projected.get("unannotated"),
        Some(&json!({"sensitive": true, "set": true}))
    );
    assert_eq!(
        projected.get("not_in_schema"),
        Some(&json!({"sensitive": true, "set": true}))
    );
}

#[test]
fn all_sensitive_schema_marks_every_field() {
    let schema = BindingsSchema::all_sensitive();
    let stored = map(json!({"proxmox_host": "https://pve.example.com:8006"}));
    let projected = schema.project(&stored);
    assert_eq!(
        projected.get("proxmox_host"),
        Some(&json!({"sensitive": true, "set": true}))
    );
}

#[test]
fn nested_secret_member_protects_the_whole_field() {
    let schema = BindingsSchema::parse(&proxmox_like_schema()).unwrap();
    assert!(schema.sensitive("nested_secret_member"));
}

#[test]
fn merge_keeps_secrets_on_null_or_omission_and_replaces_on_value() {
    let schema = BindingsSchema::parse(&proxmox_like_schema()).unwrap();
    let base = map(json!({
        "proxmox_host": "https://old.example.com:8006",
        "proxmox_token": "user@pam!id=old-secret",
        "proxmox_insecure": false
    }));
    // Sensitive omitted entirely: kept.
    let merged = schema
        .merge_update(
            &base,
            &map(json!({"proxmox_host": "https://new.example.com:8006"})),
        )
        .unwrap();
    assert_eq!(
        merged.get("proxmox_token"),
        Some(&json!("user@pam!id=old-secret"))
    );
    assert_eq!(
        merged.get("proxmox_host"),
        Some(&json!("https://new.example.com:8006"))
    );
    // Sensitive null sentinel: kept, never stored as null.
    let merged = schema
        .merge_update(&base, &map(json!({"proxmox_token": null})))
        .unwrap();
    assert_eq!(
        merged.get("proxmox_token"),
        Some(&json!("user@pam!id=old-secret"))
    );
    // Sensitive new value: replaced.
    let merged = schema
        .merge_update(
            &base,
            &map(json!({"proxmox_token": "root@pam!fresh=new-secret"})),
        )
        .unwrap();
    assert_eq!(
        merged.get("proxmox_token"),
        Some(&json!("root@pam!fresh=new-secret"))
    );
}

#[test]
fn merge_rejects_unknown_fields_and_presence_marker_echoes() {
    let schema = BindingsSchema::parse(&proxmox_like_schema()).unwrap();
    let base = map(json!({"proxmox_host": "https://old.example.com:8006"}));
    assert!(schema
        .merge_update(&base, &map(json!({"typo_field": "x"})))
        .is_err());
    assert!(schema
        .merge_update(
            &base,
            &map(json!({"proxmox_token": {"sensitive": true, "set": true}}))
        )
        .is_err());
}

#[test]
fn validate_rejects_missing_required_wrong_type_and_unknown_fields() {
    let schema = BindingsSchema::parse(&proxmox_like_schema()).unwrap();
    // Missing required token.
    let merged = map(json!({"proxmox_host": "https://pve.example.com:8006"}));
    assert!(schema.validate(&merged).is_err());
    // Wrong types.
    let merged = map(json!({
        "proxmox_host": "https://pve.example.com:8006",
        "proxmox_token": "user@pam!id=secret",
        "proxmox_vmid_begin": "not-a-number"
    }));
    assert!(schema.validate(&merged).is_err());
    // Unknown field (additionalProperties: false).
    let merged = map(json!({
        "proxmox_host": "https://pve.example.com:8006",
        "proxmox_token": "user@pam!id=secret",
        "rogue": 1
    }));
    assert!(schema.validate(&merged).is_err());
    // A full valid set passes, including bounds and pattern keywords.
    let merged = map(json!({
        "proxmox_host": "https://pve.example.com:8006",
        "proxmox_token": "user@pam!id=secret",
        "proxmox_vmid_begin": 100
    }));
    schema.validate(&merged).unwrap();
}

#[test]
fn validate_rejects_references_and_bad_annotations_fail_parse() {
    let schema = json!({"properties": {"a": {"$ref": "#/x"}}});
    let parsed = BindingsSchema::parse(&schema).unwrap();
    assert!(parsed.validate(&map(json!({"a": 1}))).is_err());
    assert!(BindingsSchema::parse(&json!({"properties": {"a": {"sensitive": "yes"}}})).is_err());
    assert!(BindingsSchema::parse(&json!("not an object")).is_err());
}

#[test]
fn combinator_branch_sensitivity_is_honored() {
    let schema = json!({
        "properties": {
            "kept": {"type": "string"},
            "via_anyof": {
                "anyOf": [
                    {"type": "string", "sensitive": true},
                    {"type": "null"}
                ]
            }
        }
    });
    let parsed = BindingsSchema::parse(&schema).unwrap();
    // An omitted annotation is conservatively sensitive; only an explicit
    // `sensitive: false` (as in the proxmox schema) exposes a value.
    assert!(parsed.sensitive("kept"));
    assert!(parsed.sensitive("via_anyof"));
}
