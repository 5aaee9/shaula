//! Projection and admission must agree on whole values without adding grammar or defaults.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use serde_json::json;

#[test]
fn fields_filter_whole_values_and_keep_native_types_without_defaults() {
    let policy = json!({
        "node": [{}, {"name":"good"}, {"name":"bad"}, {"name":"good","other":true}],
        "mixed": [null, false, 0, "", [], {}, 18446744073709551615_u64, false],
        "size_class": ["large", "small", "large", "forbidden"],
        "hidden": [1], "unapproved": []
    });
    let schema = json!({"type":"object", "required":["node"], "additionalProperties":false,
    "properties": {
        "node":{"type":"object","required":["name"],"additionalProperties":false,
            "properties":{"name":{"enum":["good"]}}},
        "mixed":{},
        "size_class":{"title":"Size", "description":"<b>plain text</b>",
            "enum":["small","large","schema-only"], "default":"schema-only"},
        "unapproved":{"type":"integer"}
    }});
    let projected = project(&policy.to_string(), &schema.to_string()).expect("projectable");
    let InputContractProjection::Fields { fields } = projected else {
        panic!("fields expected")
    };
    assert_eq!(
        fields
            .iter()
            .map(|field| field.key.as_str())
            .collect::<Vec<_>>(),
        ["mixed", "node", "size_class"]
    );
    assert!(fields[1].required);
    assert!(!fields[0].required);
    assert_eq!(fields[1].options.len(), 1);
    assert_eq!(fields[2].label, "Size");
    assert_eq!(fields[2].description, "<b>plain text</b>");
    assert_eq!(
        fields[2]
            .options
            .iter()
            .map(|o| o.value_json.as_str())
            .collect::<Vec<_>>(),
        ["\"large\"", "\"small\""]
    );
    assert_eq!(
        fields[0]
            .options
            .iter()
            .map(|o| o.value_json.as_str())
            .collect::<Vec<_>>(),
        [
            "null",
            "false",
            "0",
            "\"\"",
            "[]",
            "{}",
            "18446744073709551615"
        ]
    );
    let authority =
        InputAuthority::parse(&policy.to_string(), Some(&schema.to_string())).expect("authority");
    for field in &fields {
        for option in &field.options {
            let mut inputs = json!({"node":{"name":"good"}})
                .as_object()
                .expect("object")
                .clone();
            inputs.insert(
                field.key.clone(),
                serde_json::from_str(&option.value_json).expect("value"),
            );
            assert!(authority.validate(&inputs).is_ok());
        }
    }
    assert!(
        authority.validate(&Map::new()).is_err(),
        "default and unique option never relax required"
    );
}

#[test]
fn presets_are_only_complete_admitted_objects_and_never_cross_products() {
    let policy = r#"{"x":[1,2],"y":["a","b"]}"#;
    let schema = r#"{"enum":[{"x":1,"y":"a"},{"x":2,"y":"b"},{"x":3},{"x":1,"y":"a"},null]}"#;
    let InputContractProjection::Presets { presets } = project(policy, schema).expect("presets")
    else {
        panic!("presets expected")
    };
    assert_eq!(presets.len(), 2);
    let authority = InputAuthority::parse(policy, Some(schema)).expect("authority");
    for preset in presets {
        let inputs: Map<String, Value> = serde_json::from_str(&preset.value_json).expect("object");
        assert!(authority.validate(&inputs).is_ok());
    }
    assert!(authority
        .validate(json!({"x":1,"y":"b"}).as_object().expect("object"))
        .is_err());
    assert!(project("{}", r#"{"enum":[null,1]}"#).is_err());
}

#[test]
fn malformed_unsupported_and_unsatisfiable_authorities_do_not_become_empty_forms() {
    for (policy, schema) in [
        ("[]", "{}"),
        (r#"{"unused":1}"#, "{}"),
        ("{}", "false"),
        ("{}", r#"{"items":{}}"#),
        ("{}", r#"{"type":"array"}"#),
        ("{}", r#"{"required":["missing"]}"#),
        (
            r#"{"x":[1]}"#,
            r#"{"required":["x"],"additionalProperties":false}"#,
        ),
        (
            r#"{"x":[1]}"#,
            r#"{"required":["x"],"properties":{"x":{"type":"string"}}}"#,
        ),
    ] {
        assert!(matches!(
            project(policy, schema),
            Err(InputContractReadError::Unavailable { .. })
        ));
    }
    for schema in ["{}", r#"{"type":"object","additionalProperties":false}"#] {
        let InputContractProjection::Fields { fields } =
            project("{}", schema).expect("empty authority")
        else {
            panic!("fields expected")
        };
        assert!(fields.is_empty());
        assert!(InputAuthority::parse("{}", Some(schema))
            .expect("authority")
            .validate(&Map::new())
            .is_ok());
    }
}

#[test]
fn projection_limits_never_truncate_or_change_the_admission_range() {
    let policy: Map<String, Value> = (0..257).map(|i| (format!("k{i}"), json!([i]))).collect();
    assert!(project(&Value::Object(policy.clone()).to_string(), "{}").is_err());
    let thirty_three: Vec<String> = (0..33).map(|i| format!("k{i}")).collect();
    assert!(project(
        &Value::Object(policy).to_string(),
        &json!({"required":thirty_three}).to_string()
    )
    .is_err());
    let many: Vec<u64> = (0..257).collect();
    assert!(project(&json!({"x":many}).to_string(), "{}").is_err());
    let mut deep = json!(false);
    for _ in 0..17 {
        deep = json!([deep]);
    }
    let deep_policy = json!({"x":[deep.clone()]}).to_string();
    assert!(project(&deep_policy, "{}").is_err());
    assert!(InputAuthority::parse(&deep_policy, Some("{}"))
        .expect("authority")
        .validate(json!({"x":deep}).as_object().expect("object"))
        .is_ok());
    let over_budget = json!({"x":["a".repeat(600_000),"b".repeat(600_000)]}).to_string();
    assert!(project(&over_budget, "{}").is_err());
    let thirty_two: Vec<String> = (0..32).map(|i| format!("k{i}")).collect();
    let approved: Map<String, Value> = thirty_two
        .iter()
        .map(|key| (key.clone(), json!([true])))
        .collect();
    assert!(project(
        &Value::Object(approved).to_string(),
        &json!({"required":thirty_two}).to_string()
    )
    .is_ok());
}
