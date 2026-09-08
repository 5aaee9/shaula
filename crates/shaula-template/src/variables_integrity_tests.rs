use super::discover_variables;
use super::tests::fixture;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn numeric_defaults_that_hcl_would_round_or_overflow_are_rejected() -> TestResult {
    for value in [
        "1e20",
        "9007199254740993.0",
        "0.12345678901234567890123456789",
        "1e-1000",
        "-18446744073709551615",
        "--9223372036854775808",
        "1e308",
    ] {
        let directory = fixture(
            &format!("value = optional(number, {value})"),
            r#"{"type":"object","properties":{"value":{"type":"number"}}}"#,
        )?;
        let error = discover_variables(directory.path(), "digest")
            .err()
            .ok_or("lossy number accepted")?;
        assert!(
            !error.summary.contains(value),
            "numeric literal leaked in error"
        );
    }
    let directory = fixture(
        "value = optional(list(number), [1e20])",
        r#"{"type":"object","properties":{"value":{"type":"array"}}}"#,
    )?;
    assert!(discover_variables(directory.path(), "digest").is_err());
    Ok(())
}

#[test]
fn schema_options_and_defaults_cannot_silently_round_numbers() -> TestResult {
    for value in [
        "9007199254740993.0",
        "0.12345678901234567890123456789",
        "1e-1000",
    ] {
        let directory = fixture(
            "value = optional(number)",
            &format!(
                r#"{{"type":"object","properties":{{"value":{{"type":"number","enum":[{value}]}}}}}}"#
            ),
        )?;
        let error = discover_variables(directory.path(), "digest")
            .err()
            .ok_or("lossy schema number accepted")?;
        assert!(!error.summary.contains(value));
    }
    let directory = fixture(
        "value = optional(number, 9007199254740992)",
        r#"{"type":"object","properties":{"value":{"type":"number","default":9007199254740993.0}}}"#,
    )?;
    assert!(discover_variables(directory.path(), "digest").is_err());
    let directory = fixture(
        "value = optional(list(number))",
        r#"{"type":"object","properties":{"value":{"type":"array","enum":[[9007199254740993.0]]}}}"#,
    )?;
    assert!(discover_variables(directory.path(), "digest").is_err());
    let directory = fixture(
        "value = optional(number)",
        r#"{"type":"object","properties":{"value":{"type":"number","enum":[1e20,9007199254740993,0.10]}}}"#,
    )?;
    let result = discover_variables(directory.path(), "digest")?;
    assert_eq!(result.parameters[0].options.len(), 3);
    assert_eq!(result.parameters[0].options[0].value_json, "1e+20");
    assert_eq!(
        result.parameters[0].options[1].value_json,
        "9007199254740993"
    );
    assert_eq!(result.parameters[0].options[2].value_json, "0.1");
    Ok(())
}

#[test]
fn exactly_equivalent_numeric_spellings_and_integer_edges_are_preserved() -> TestResult {
    for (value, expected) in [
        ("1e3", "1000"),
        ("9007199254740993", "9007199254740993"),
        ("18446744073709551615", "18446744073709551615"),
        ("-9223372036854775808", "-9223372036854775808"),
        ("0.10", "0.1"),
        ("-0.1", "-0.1"),
        ("1e-3", "0.001"),
    ] {
        let directory = fixture(
            &format!("value = optional(number, {value})"),
            r#"{"type":"object","properties":{"value":{"type":"number"}}}"#,
        )?;
        let result = discover_variables(directory.path(), "digest")?;
        assert_eq!(
            result.parameters[0].default_value_json.as_deref(),
            Some(expected)
        );
    }
    Ok(())
}

#[test]
fn unrelated_module_numbers_do_not_enter_the_discovery_conversion() -> TestResult {
    let directory = fixture("", r#"{"type":"object","properties":{}}"#)?;
    std::fs::write(
        directory.path().join("main.tf"),
        "locals {\n value = --9223372036854775808\n}\n",
    )?;
    assert!(discover_variables(directory.path(), "digest")?.available);
    Ok(())
}

#[test]
fn typed_collection_null_members_remain_subject_to_schema_constraints() -> TestResult {
    for (declaration, schema, expected) in [
        (
            "value = optional(list(string), [null])",
            r#"{"type":"array","enum":[[null]]}"#,
            "[null]",
        ),
        (
            "value = optional(tuple([string,bool]), [\"a\",null])",
            r#"{"type":"array","enum":[["a",null]]}"#,
            "[\"a\",null]",
        ),
        (
            "value = optional(map(string), {a = null})",
            r#"{"type":"object","enum":[{"a":null}]}"#,
            "{\"a\":null}",
        ),
    ] {
        let directory = fixture(
            declaration,
            &format!(r#"{{"type":"object","properties":{{"value":{schema}}}}}"#),
        )?;
        let result = discover_variables(directory.path(), "digest")?;
        assert_eq!(
            result.parameters[0].default_value_json.as_deref(),
            Some(expected)
        );
    }
    let directory = fixture(
        "value = optional(list(string), [null])",
        r#"{"type":"object","properties":{"value":{"type":"array","items":{"type":"string"}}}}"#,
    )?;
    assert!(discover_variables(directory.path(), "digest").is_err());
    Ok(())
}

#[test]
fn group_sensitivity_and_compositions_are_inherited_without_sibling_taint() -> TestResult {
    for marker in [
        r#""sensitive":true"#,
        r#""anyOf":[{"sensitive":true}]"#,
        r#""allOf":[{"sensitive":true}]"#,
    ] {
        let schema = format!(
            r#"{{"type":"object",{marker},"properties":{{"value":{{"type":"string","sensitive":false,"enum":["private-token"]}}}}}}"#
        );
        let directory = fixture(r#"value = optional(string, "private-token")"#, &schema)?;
        let result = discover_variables(directory.path(), "digest")?;
        assert!(result.parameters[0].sensitive);
        assert!(!serde_json::to_string(&result)?.contains("private-token"));
    }
    let directory = fixture(
        r#"
        private = optional(string, "private-token")
        public = optional(string, "public-value")
    "#,
        r#"{"type":"object","properties":{
        "private":{"type":"string","sensitive":true},
        "public":{"type":"string","sensitive":false}
    }}"#,
    )?;
    let result = discover_variables(directory.path(), "digest")?;
    assert!(result.parameters[0].sensitive);
    assert!(!result.parameters[1].sensitive);
    assert_eq!(
        result.parameters[1].default_value_json.as_deref(),
        Some("\"public-value\"")
    );
    Ok(())
}
