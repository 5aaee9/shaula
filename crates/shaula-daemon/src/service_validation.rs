//! Pure admission validators: bounded input policy and full
//! attestation-subject recomputation (spec 0002 §4.1, 0005 §5.1).

use shaula_core::error::{CoreError, CoreResult, ReasonCode};

/// Validates Fleet template inputs against BOTH authorities (spec 0002
/// §4.1, 0004 §3): the artifact's declared parameter schema (required
/// keys, JSON types, enum bounds) and the pinned revision's finite alias
/// policy. Unknown keys, missing required keys, wrong types and
/// out-of-alias values fail admission; empty inputs still hit the
/// required check.
pub(crate) fn validate_inputs(
    inputs: &serde_json::Map<String, serde_json::Value>,
    policy_json: &str,
    schema_json: Option<&str>,
) -> CoreResult<()> {
    let policy: serde_json::Value = serde_json::from_str(policy_json)
        .map_err(|e| CoreError::new(ReasonCode::SpecInvalid, format!("policy invalid: {e}")))?;
    let Some(policy_obj) = policy.as_object() else {
        return Err(CoreError::new(
            ReasonCode::SpecInvalid,
            "policy must be an object",
        ));
    };
    if inputs.len() > 32 {
        return Err(CoreError::new(ReasonCode::SpecInvalid, "too many inputs"));
    }

    // Layer 1: the artifact's declared parameter schema — required keys
    // and types. Supported grammar is an ALLOWLIST (type / properties /
    // required / enum / additionalProperties plus annotations): anything
    // else (constraints, combinators, refs, boolean schemas) fails
    // admission instead of silently degrading to "unconstrained"
    // (spec 0002 §4.1, 0004 §3). Property NAMES are data, never keywords:
    // an input literally named "maximum" validates normally.
    if let Some(schema_json) = schema_json {
        const SUPPORTED_SCHEMA_KEYWORDS: &[&str] = &[
            "$schema",
            "type",
            "properties",
            "required",
            "enum",
            "additionalProperties",
            "description",
            "title",
            "default",
        ];
        /// The `$schema` dialect declaration (both bundled profiles ship
        /// it): only the standard JSON Schema dialect URIs are accepted
        /// (R5-04).
        fn supported_dialect(value: &serde_json::Value) -> CoreResult<()> {
            let uri = value.as_str().ok_or_else(|| {
                CoreError::new(ReasonCode::SpecInvalid, "schema $schema must be a string")
            })?;
            if uri.starts_with("http://json-schema.org/")
                || uri.starts_with("https://json-schema.org/")
                || uri.starts_with("https://json-schema.org/draft/")
            {
                Ok(())
            } else {
                Err(CoreError::new(
                    ReasonCode::SpecInvalid,
                    format!("schema dialect {uri:?} is unsupported"),
                ))
            }
        }
        fn schema_shape(node: &serde_json::Value) -> CoreResult<()> {
            let Some(obj) = node.as_object() else {
                return Err(CoreError::new(
                    ReasonCode::SpecInvalid,
                    "parameter schema must be an object; boolean/unknown schemas are unsupported",
                ));
            };
            for key in obj.keys() {
                if !SUPPORTED_SCHEMA_KEYWORDS.contains(&key.as_str()) {
                    return Err(CoreError::new(
                        ReasonCode::SpecInvalid,
                        format!(
                            "parameter schema uses unsupported constraint {key:?}; the profile must declare a supported subset"
                        ),
                    ));
                }
            }
            if let Some(dialect) = obj.get("$schema") {
                supported_dialect(dialect)?;
            }
            if let Some(enum_values) = obj.get("enum") {
                // R5-04: a non-array `enum` (e.g. 42) constrained nothing
                // at validation time — it must be rejected as malformed.
                if enum_values.as_array().is_none() {
                    return Err(CoreError::new(
                        ReasonCode::SpecInvalid,
                        "schema enum must be an array",
                    ));
                }
            }
            if let Some(t) = obj.get("type") {
                let t = t.as_str().ok_or_else(|| {
                    CoreError::new(ReasonCode::SpecInvalid, "schema type must be a string")
                })?;
                if !matches!(
                    t,
                    "object" | "string" | "integer" | "number" | "boolean" | "array"
                ) {
                    return Err(CoreError::new(
                        ReasonCode::SpecInvalid,
                        format!("schema type {t:?} is unsupported"),
                    ));
                }
            }
            if let Some(required) = obj.get("required") {
                let Some(required) = required.as_array() else {
                    return Err(CoreError::new(
                        ReasonCode::SpecInvalid,
                        "schema required must be an array",
                    ));
                };
                if required.iter().any(|r| !r.is_string()) {
                    return Err(CoreError::new(
                        ReasonCode::SpecInvalid,
                        "schema required entries must be strings",
                    ));
                }
            }
            if let Some(additional) = obj.get("additionalProperties") {
                if !additional.is_boolean() {
                    return Err(CoreError::new(
                        ReasonCode::SpecInvalid,
                        "schema additionalProperties must be a boolean",
                    ));
                }
            }
            if let Some(properties) = obj.get("properties") {
                let Some(properties) = properties.as_object() else {
                    return Err(CoreError::new(
                        ReasonCode::SpecInvalid,
                        "schema properties must be an object",
                    ));
                };
                // Recurse into property SUBSCHEMAS. enum/default values
                // are data and are deliberately never traversed as
                // schemas.
                for subschema in properties.values() {
                    schema_shape(subschema)?;
                }
            }
            Ok(())
        }
        fn type_matches(declared: &str, value: &serde_json::Value) -> bool {
            match declared {
                "string" => value.is_string(),
                "integer" => value.is_i64() || value.is_u64(),
                "number" => value.is_number(),
                "boolean" => value.is_boolean(),
                "object" => value.is_object(),
                "array" => value.is_array(),
                _ => true,
            }
        }
        /// Validates one input value against its schema node: type,
        /// enum, required keys and additionalProperties at every nested
        /// object level (a nested `required: [name]` with `{}` must
        /// reject — an unenforced nested constraint is a hole, not a
        /// feature).
        fn validate_value(
            path: &str,
            value: &serde_json::Value,
            schema: &serde_json::Value,
        ) -> CoreResult<()> {
            let Some(obj) = schema.as_object() else {
                return Ok(());
            };
            if let Some(declared) = obj.get("type").and_then(|t| t.as_str()) {
                if !type_matches(declared, value) {
                    return Err(CoreError::new(
                        ReasonCode::SpecInvalid,
                        format!("input {path:?} does not match the declared type {declared:?}"),
                    ));
                }
            }
            if let Some(enum_values) = obj.get("enum").and_then(|v| v.as_array()) {
                if !enum_values.contains(value) {
                    return Err(CoreError::new(
                        ReasonCode::SpecInvalid,
                        format!("input {path:?} is outside the profile's bounded values"),
                    ));
                }
            }
            if value.is_object() {
                for key in obj
                    .get("required")
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten()
                {
                    let key = key.as_str().unwrap_or_default();
                    if !value.as_object().is_some_and(|v| v.contains_key(key)) {
                        return Err(CoreError::new(
                            ReasonCode::SpecInvalid,
                            format!("required input {key:?} is missing"),
                        ));
                    }
                }
                let properties = obj.get("properties").and_then(|v| v.as_object());
                if obj.get("additionalProperties").and_then(|v| v.as_bool()) == Some(false) {
                    for key in value.as_object().map(|v| v.keys()).into_iter().flatten() {
                        if !properties.is_some_and(|p| p.contains_key(key.as_str())) {
                            return Err(CoreError::new(
                                ReasonCode::SpecInvalid,
                                format!("input {key:?} is not declared by the profile schema"),
                            ));
                        }
                    }
                }
                for (key, subschema) in properties.into_iter().flatten() {
                    if let Some(inner) = value.get(key) {
                        let nested = if path.is_empty() {
                            key.clone()
                        } else {
                            format!("{path}.{key}")
                        };
                        validate_value(&nested, inner, subschema)?;
                    }
                }
            }
            Ok(())
        }
        let schema: serde_json::Value = serde_json::from_str(schema_json)
            .map_err(|e| CoreError::new(ReasonCode::SpecInvalid, format!("schema invalid: {e}")))?;
        schema_shape(&schema)?;
        let input_value = serde_json::Value::Object(inputs.clone());
        validate_value("", &input_value, &schema)?;
    }

    // Layer 2: the pinned revision's finite alias policy.
    for (key, value) in inputs {
        let Some(allowed) = policy_obj.get(key).and_then(|v| v.as_array()) else {
            return Err(CoreError::new(
                ReasonCode::SpecInvalid,
                format!("input {key:?} is not in the profile input policy"),
            ));
        };
        if !allowed.contains(value) {
            return Err(CoreError::new(
                ReasonCode::SpecInvalid,
                format!("input {key:?} is not an allowed alias"),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "service_validation_tests.rs"]
mod tests;
