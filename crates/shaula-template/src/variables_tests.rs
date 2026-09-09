use super::discover_variables;

type TestResult = Result<(), Box<dyn std::error::Error>>;

pub(super) fn fixture(
    parameters: &str,
    schema: &str,
) -> Result<tempfile::TempDir, Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    std::fs::create_dir(directory.path().join("schemas"))?;
    std::fs::write(
        directory.path().join("variables.tf"),
        format!(
            r#"
variable "shaula" {{
  sensitive = true
  type = object({{
    contract_version = number
    generation = any
    jit_config = string
    bindings_digest = string
    bindings = object({{}})
    parameters = object({{ {parameters} }})
  }})
}}
"#
        ),
    )?;
    std::fs::write(
        directory.path().join("schemas/bindings.schema.json"),
        r#"{"type":"object","properties":{}}"#,
    )?;
    std::fs::write(
        directory.path().join("schemas/parameters.schema.json"),
        schema,
    )?;
    Ok(directory)
}

#[test]
fn bundled_variables_are_discovered_from_terraform_defaults() -> TestResult {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates");
    let docker = discover_variables(&root.join("docker"), "digest")?;
    assert!(docker.available);
    assert_eq!(docker.parameters.len(), 1);
    assert_eq!(
        docker.parameters[0].default_value_json.as_deref(),
        Some("\"ghcr.io/actions/actions-runner:2.337.0\"")
    );
    let socket = docker
        .bindings
        .iter()
        .find(|field| field.key == "docker_host")
        .ok_or("socket missing")?;
    assert!(socket.required);
    assert_eq!(
        socket.default_value_json.as_deref(),
        Some("\"unix:///var/run/docker.sock\"")
    );
    let credential = docker
        .bindings
        .iter()
        .find(|field| field.key == "registry_auth")
        .ok_or("credential missing")?;
    assert!(credential.sensitive);
    assert!(credential.default_value_json.is_none());
    let kubernetes = discover_variables(&root.join("kubernetes"), "digest")?;
    assert_eq!(kubernetes.parameters.len(), 3);
    assert_eq!(kubernetes.parameters[0].key, "cpu_request");
    assert_eq!(
        kubernetes.parameters[0].default_value_json.as_deref(),
        Some("\"500m\"")
    );
    assert_eq!(
        kubernetes.parameters[1].default_value_json.as_deref(),
        Some("\"2Gi\"")
    );
    assert!(kubernetes.bindings.iter().all(|field| field.required));
    Ok(())
}

#[test]
fn literals_preserve_zero_false_empty_string_and_large_integer() -> TestResult {
    let directory = fixture(
        r#"
      enabled = optional(bool, false)
      count = optional(number, 0)
      big = optional(number, 9007199254740993)
      name = optional(string, "")
    "#,
        r#"{"type":"object","properties":{
      "enabled":{"type":"boolean","enum":[false,true]},
      "count":{"type":"integer","enum":[0,1]},
      "big":{"type":"integer","enum":[9007199254740993]},
      "name":{"type":"string","enum":[""]}
    }}"#,
    )?;
    let result = discover_variables(directory.path(), "digest")?;
    let values: Vec<_> = result
        .parameters
        .iter()
        .map(|field| (field.key.as_str(), field.default_value_json.as_deref()))
        .collect();
    assert_eq!(
        values,
        vec![
            ("big", Some("9007199254740993")),
            ("count", Some("0")),
            ("enabled", Some("false")),
            ("name", Some("\"\""))
        ]
    );
    Ok(())
}

#[test]
fn legacy_any_is_explicitly_unavailable_and_does_not_read_schema() -> TestResult {
    let directory = tempfile::tempdir()?;
    std::fs::write(
        directory.path().join("main.tf"),
        "variable \"shaula\" {\n type = any\n sensitive = true\n}\n",
    )?;
    let result = discover_variables(directory.path(), "digest")?;
    assert!(!result.available);
    assert!(result.reason.is_some());
    assert!(result.bindings.is_empty() && result.parameters.is_empty());
    Ok(())
}

#[test]
fn sensitive_defaults_and_options_never_leave_the_projection() -> TestResult {
    let directory = fixture(
        r#"credential = optional(string, "private-token")"#,
        r#"{"type":"object","properties":{"credential":{"type":"string","sensitive":true,"enum":["private-token"]}}}"#,
    )?;
    let result = discover_variables(directory.path(), "digest")?;
    assert!(result.parameters[0].sensitive);
    assert!(result.parameters[0].default_value_json.is_none());
    assert!(result.parameters[0].options.is_empty());
    assert!(!serde_json::to_string(&result)?.contains("private-token"));
    Ok(())
}

#[test]
fn nested_secret_marks_whole_object_protected() -> TestResult {
    let directory = fixture(
        r#"config = optional(object({ token = string }), {token = "private-token"})"#,
        r#"{"type":"object","properties":{"config":{"type":"object","sensitive":false,"required":["token"],"properties":{"token":{"type":"string","sensitive":true}},"enum":[{"token":"private-token"}]}}}"#,
    )?;
    let result = discover_variables(directory.path(), "digest")?;
    assert!(result.parameters[0].sensitive);
    assert!(!serde_json::to_string(&result)?.contains("private-token"));
    Ok(())
}

#[test]
fn defaults_must_be_literal_and_schema_agreement_is_strict() -> TestResult {
    for (declaration, schema) in [
        (
            "value = optional(string, file(\"secret\"))",
            r#"{"type":"string"}"#,
        ),
        (
            "value = optional(string, \"${local.secret}\")",
            r#"{"type":"string"}"#,
        ),
        ("value = optional(string, \"a\")", r#"{"type":"number"}"#),
        (
            "value = optional(string, \"a\")",
            r#"{"type":"string","default":"b"}"#,
        ),
        (
            "value = optional(string, \"a\")",
            r#"{"type":"string","enum":["b"]}"#,
        ),
        ("value = optional(number, \"1\")", r#"{"type":"number"}"#),
        ("value = string", r#"{"type":"string"}"#),
        ("value = optional(set(string), [])", r#"{"type":"array"}"#),
    ] {
        let directory = fixture(
            declaration,
            &format!(r#"{{"type":"object","properties":{{"value":{schema}}}}}"#),
        )?;
        assert!(
            discover_variables(directory.path(), "digest").is_err(),
            "{declaration}"
        );
    }
    Ok(())
}

#[test]
fn root_sources_reject_duplicate_variables_and_system_member_changes() -> TestResult {
    let directory = fixture("", r#"{"type":"object","properties":{}}"#)?;
    std::fs::write(
        directory.path().join("extra.tf"),
        "variable \"other\" { type = string }",
    )?;
    assert!(discover_variables(directory.path(), "digest").is_err());
    std::fs::remove_file(directory.path().join("extra.tf"))?;
    let path = directory.path().join("variables.tf");
    let source = std::fs::read_to_string(&path)?;
    for changed in [
        source.replace("sensitive = true", "sensitive = false"),
        source.replace(
            "jit_config = string",
            "jit_config = optional(string, \"secret\")",
        ),
        source.replace(
            "bindings_digest = string",
            "bindings_digest = string\n another = string",
        ),
    ] {
        std::fs::write(&path, changed)?;
        assert!(discover_variables(directory.path(), "digest").is_err());
    }
    Ok(())
}

#[test]
fn source_and_projection_limits_fail_without_truncation() -> TestResult {
    let directory = fixture("", r#"{"type":"object","properties":{}}"#)?;
    std::fs::write(
        directory.path().join("big.tf"),
        " ".repeat(super::MAX_BYTES + 1),
    )?;
    assert!(discover_variables(directory.path(), "digest").is_err());
    std::fs::remove_file(directory.path().join("big.tf"))?;
    let options = (0..257).collect::<Vec<_>>();
    std::fs::write(directory.path().join("schemas/parameters.schema.json"), serde_json::json!({"type":"object","properties":{"value":{"type":"number","enum":options}}}).to_string())?;
    let path = directory.path().join("variables.tf");
    std::fs::write(
        &path,
        std::fs::read_to_string(&path)?.replace(
            "parameters = object({  })",
            "parameters = object({ value = optional(number, 0) })",
        ),
    )?;
    assert!(discover_variables(directory.path(), "digest").is_err());
    Ok(())
}

#[test]
fn lists_maps_tuples_and_objects_keep_whole_literal_values() -> TestResult {
    let directory = fixture(
        r#"
      list = optional(list(string), ["a", "b"])
      map = optional(map(number), {"one" = 1})
      tuple = optional(tuple([string, bool]), ["a", false])
      object = optional(object({label = string}), {label = "a"})
    "#,
        r#"{"type":"object","properties":{
      "list":{"type":"array","enum":[["a","b"]]},
      "map":{"type":"object","enum":[{"one":1}]},
      "tuple":{"type":"array","enum":[["a",false]]},
      "object":{"type":"object","required":["label"],"properties":{"label":{"type":"string"}},"enum":[{"label":"a"}]}
    }}"#,
    )?;
    let result = discover_variables(directory.path(), "digest")?;
    assert_eq!(
        result.parameters[0].default_value_json.as_deref(),
        Some("[\"a\",\"b\"]")
    );
    assert_eq!(
        result.parameters[1].default_value_json.as_deref(),
        Some("{\"one\":1}")
    );
    assert_eq!(
        result.parameters[2].default_value_json.as_deref(),
        Some("{\"label\":\"a\"}")
    );
    assert_eq!(
        result.parameters[3].default_value_json.as_deref(),
        Some("[\"a\",false]")
    );
    Ok(())
}

#[test]
fn negative_numeric_literal_is_supported() -> TestResult {
    let directory = fixture(
        "value = optional(number, -1)",
        r#"{"type":"object","properties":{"value":{"type":"number","enum":[-1,0]}}}"#,
    )?;
    let result = discover_variables(directory.path(), "digest")?;
    assert_eq!(
        result.parameters[0].default_value_json.as_deref(),
        Some("-1")
    );
    Ok(())
}

#[test]
fn explicit_null_default_is_distinct_from_no_declared_default() -> TestResult {
    let directory = fixture(
        "implicit = optional(string)\n explicit = optional(string, null)",
        r#"{"type":"object","properties":{"implicit":{"type":"string"},"explicit":{"type":"string"}}}"#,
    )?;
    let result = discover_variables(directory.path(), "digest")?;
    assert_eq!(
        result.parameters[0].default_value_json.as_deref(),
        Some("null")
    );
    assert!(result.parameters[1].default_value_json.is_none());
    assert!(result
        .parameters
        .iter()
        .all(|field| !field.required && field.options.is_empty()));
    Ok(())
}

#[test]
fn ambiguous_object_members_and_override_files_are_rejected() -> TestResult {
    let directory = fixture(
        "value = optional(string, \"a\")\n value = optional(string, \"b\")",
        r#"{"type":"object","properties":{"value":{"type":"string"}}}"#,
    )?;
    assert!(discover_variables(directory.path(), "digest").is_err());
    let directory = fixture("", r#"{"type":"object","properties":{}}"#)?;
    for name in ["main.tf.json", "override.tf", "local_override.tf"] {
        std::fs::write(directory.path().join(name), "{}")?;
        assert!(discover_variables(directory.path(), "digest").is_err());
        std::fs::remove_file(directory.path().join(name))?;
    }
    Ok(())
}
