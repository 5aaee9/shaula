use super::discover_variables;
use super::tests::fixture;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn default_rejected_by_string_constraints_cannot_be_suggested() -> TestResult {
    for (value, constraint) in [
        ("", r#""minLength":1"#),
        ("long", r#""maxLength":3"#),
        ("wrong", r#""pattern":"^right$""#),
        ("not a uri", r#""format":"uri""#),
        (
            "tcp://localhost:2375",
            r#""anyOf":[{"format":"uri","pattern":"^unix://"},{"format":"uri","pattern":"^npipe://"}]"#,
        ),
        ("a", r#""allOf":[{"minLength":1},{"pattern":"^b$"}]"#),
        ("a", r#""oneOf":[{"minLength":1},{"pattern":"^a$"}]"#),
        ("a", r#""not":{"enum":["a"]}"#),
    ] {
        let declaration = format!(
            "value = optional(string, {})",
            serde_json::to_string(value)?
        );
        let schema = format!(
            r#"{{"type":"object","properties":{{"value":{{"type":"string",{constraint}}}}}}}"#
        );
        let directory = fixture(&declaration, &schema)?;
        assert!(
            discover_variables(directory.path(), "digest").is_err(),
            "{constraint}"
        );
    }
    Ok(())
}

#[test]
fn valid_uri_and_unicode_length_defaults_are_discovered() -> TestResult {
    let directory = fixture(
        r#"
        value = optional(string, "unix:///var/run/docker.sock")
        label = optional(string, "你好")
    "#,
        r#"{"type":"object","properties":{
        "value":{"type":"string","anyOf":[{"format":"uri","pattern":"^unix://"},{"format":"uri","pattern":"^npipe://"}]},
        "label":{"type":"string","minLength":2,"maxLength":2}
    }}"#,
    )?;
    let result = discover_variables(directory.path(), "digest")?;
    assert_eq!(result.parameters.len(), 2);
    Ok(())
}

#[test]
fn invalid_options_and_unknown_constraints_fail_without_default_values() -> TestResult {
    for constraint in [
        r#""pattern":"^a$","enum":["b"]"#,
        r#""pattern":"[""#,
        r#""minLength":"1""#,
        r#""$ref":"file:///tmp/private-variable-source.json""#,
        r#""format":"unknown-format""#,
    ] {
        let schema = format!(
            r#"{{"type":"object","properties":{{"value":{{"type":"string",{constraint}}}}}}}"#
        );
        let directory = fixture("value = optional(string)", &schema)?;
        assert!(
            discover_variables(directory.path(), "digest").is_err(),
            "{constraint}"
        );
    }
    Ok(())
}

#[test]
fn collection_element_schema_drift_fails_even_with_empty_defaults() -> TestResult {
    for (declaration, schema) in [
        (
            "value = optional(list(string), [])",
            r#"{"type":"array","items":{"type":"number"}}"#,
        ),
        (
            "value = optional(tuple([string, bool]), [\"a\",false])",
            r#"{"type":"array","prefixItems":[{"type":"string"},{"type":"number"}]}"#,
        ),
        (
            "value = optional(tuple([string, bool]), [\"a\",false])",
            r#"{"type":"array","prefixItems":[{"type":"string"}]}"#,
        ),
        (
            "value = optional(map(string), {})",
            r#"{"type":"object","additionalProperties":{"type":"number"}}"#,
        ),
    ] {
        let schema = format!(r#"{{"type":"object","properties":{{"value":{schema}}}}}"#);
        let directory = fixture(declaration, &schema)?;
        assert!(
            discover_variables(directory.path(), "digest").is_err(),
            "{declaration}"
        );
    }
    Ok(())
}

#[test]
fn collection_defaults_obey_element_and_size_constraints() -> TestResult {
    for (declaration, schema) in [
        (
            "value = optional(list(string), [\"a\"])",
            r#"{"type":"array","items":{"type":"string","minLength":2}}"#,
        ),
        (
            "value = optional(list(string), [])",
            r#"{"type":"array","minItems":1}"#,
        ),
        (
            "value = optional(list(string), [\"a\",\"a\"])",
            r#"{"type":"array","uniqueItems":true}"#,
        ),
        (
            "value = optional(map(string), {})",
            r#"{"type":"object","minProperties":1}"#,
        ),
    ] {
        let schema = format!(r#"{{"type":"object","properties":{{"value":{schema}}}}}"#);
        let directory = fixture(declaration, &schema)?;
        assert!(
            discover_variables(directory.path(), "digest").is_err(),
            "{declaration}"
        );
    }
    Ok(())
}

#[test]
fn sensitive_composition_metadata_protects_the_whole_variable() -> TestResult {
    let directory = fixture(
        r#"value = optional(string, "private-token")"#,
        r#"{"type":"object","properties":{"value":{"type":"string","anyOf":[{"sensitive":true,"enum":["private-token"]}]}}}"#,
    )?;
    let result = discover_variables(directory.path(), "digest")?;
    assert!(result.parameters[0].sensitive);
    assert!(!serde_json::to_string(&result)?.contains("private-token"));
    Ok(())
}
