//! Supported schema constraints for declared defaults and publisher option suggestions.

use serde_json::Value;
use shaula_core::error::CoreResult;

use super::{invalid, MAX_DEPTH, MAX_FIELDS};

const KEYWORDS: &[&str] = &[
    "$schema",
    "title",
    "description",
    "default",
    "sensitive",
    "type",
    "enum",
    "required",
    "properties",
    "additionalProperties",
    "items",
    "prefixItems",
    "minItems",
    "maxItems",
    "uniqueItems",
    "minProperties",
    "maxProperties",
    "minLength",
    "maxLength",
    "pattern",
    "format",
    "anyOf",
    "allOf",
    "oneOf",
    "not",
];

fn pattern(value: &Value) -> CoreResult<regex::Regex> {
    let value = value
        .as_str()
        .filter(|value| value.len() <= 4096)
        .ok_or_else(|| invalid("variable schema pattern must be a bounded string"))?;
    regex::RegexBuilder::new(value)
        .size_limit(1024 * 1024)
        .build()
        .map_err(|_| invalid("variable schema pattern is invalid or unsupported"))
}

pub(super) fn check(schema: &Value, depth: usize) -> CoreResult<()> {
    let object = schema
        .as_object()
        .filter(|_| depth <= MAX_DEPTH)
        .ok_or_else(|| invalid("variable schema is invalid or too deep"))?;
    for (key, value) in object {
        if !KEYWORDS.contains(&key.as_str()) {
            return Err(invalid("variable schema uses an unsupported constraint"));
        }
        match key.as_str() {
            "type"
                if !value.as_str().is_some_and(|name| {
                    matches!(
                        name,
                        "string" | "number" | "integer" | "boolean" | "array" | "object" | "null"
                    )
                }) =>
            {
                return Err(invalid("variable schema type is invalid or unsupported"));
            }
            "minLength" | "maxLength" | "minItems" | "maxItems" | "minProperties"
            | "maxProperties" => {
                if value.as_u64().is_none() {
                    return Err(invalid(
                        "variable schema bound must be a nonnegative integer",
                    ));
                }
            }
            "pattern" => {
                pattern(value)?;
            }
            "format" if value.as_str() != Some("uri") => {
                return Err(invalid("variable schema format is unsupported"));
            }
            "sensitive" | "uniqueItems" if !value.is_boolean() => {
                return Err(invalid("variable schema annotation must be boolean"));
            }
            "properties" => {
                let fields = value
                    .as_object()
                    .filter(|fields| fields.len() <= MAX_FIELDS)
                    .ok_or_else(|| {
                        invalid("variable schema properties are invalid or too large")
                    })?;
                for value in fields.values() {
                    check(value, depth + 1)?;
                }
            }
            "additionalProperties" | "items" if value.is_boolean() => {}
            "items" if value.is_array() => {
                for value in array(value)? {
                    check(value, depth + 1)?;
                }
            }
            "additionalProperties" | "items" | "not" => check(value, depth + 1)?,
            "prefixItems" | "anyOf" | "allOf" | "oneOf" => {
                let branches = array(value)?;
                if branches.is_empty() && key != "prefixItems" {
                    return Err(invalid("variable schema composition must not be empty"));
                }
                for value in branches {
                    check(value, depth + 1)?;
                }
            }
            "enum" => {
                array(value)?;
            }
            "required" => {
                let mut names = std::collections::BTreeSet::new();
                for value in array(value)? {
                    let name = value
                        .as_str()
                        .ok_or_else(|| invalid("variable schema required names must be strings"))?;
                    if !names.insert(name) {
                        return Err(invalid("variable schema required contains duplicate names"));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn array(value: &Value) -> CoreResult<&[Value]> {
    value
        .as_array()
        .filter(|values| values.len() <= MAX_FIELDS)
        .map(Vec::as_slice)
        .ok_or_else(|| invalid("variable schema array is invalid or exceeds the item limit"))
}

fn bounds(schema: &Value, min: &str, max: &str, size: usize) -> bool {
    let size = size as u64;
    schema
        .get(min)
        .and_then(Value::as_u64)
        .is_none_or(|min| size >= min)
        && schema
            .get(max)
            .and_then(Value::as_u64)
            .is_none_or(|max| size <= max)
}

/// Schema shape is checked first; recursively test only this explicitly supported vocabulary.
pub(super) fn accepts(schema: &Value, value: &Value, depth: usize) -> CoreResult<bool> {
    if depth > MAX_DEPTH {
        return Err(invalid("variable schema value exceeds the depth limit"));
    }
    if let Some(constant) = schema.as_bool() {
        return Ok(constant);
    }
    let type_matches = match schema.get("type").and_then(Value::as_str) {
        Some("string") => value.is_string(),
        Some("number") => value.is_number(),
        Some("integer") => value.is_i64() || value.is_u64(),
        Some("boolean") => value.is_boolean(),
        Some("object") => value.is_object(),
        Some("array") => value.is_array(),
        Some("null") => value.is_null(),
        None => true,
        _ => false,
    };
    if !type_matches
        || schema.get("enum").is_some_and(|options| {
            !options
                .as_array()
                .is_some_and(|options| options.contains(value))
        })
    {
        return Ok(false);
    }
    for keyword in ["anyOf", "allOf", "oneOf"] {
        if let Some(branches) = schema.get(keyword) {
            let branches = array(branches)?;
            let mut count = 0;
            for branch in branches {
                count += usize::from(accepts(branch, value, depth + 1)?);
            }
            let valid = match keyword {
                "anyOf" => count > 0,
                "allOf" => count == branches.len(),
                _ => count == 1,
            };
            if !valid {
                return Ok(false);
            }
        }
    }
    if let Some(negated) = schema.get("not") {
        if accepts(negated, value, depth + 1)? {
            return Ok(false);
        }
    }
    if let Some(text) = value.as_str() {
        if !bounds(schema, "minLength", "maxLength", text.chars().count()) {
            return Ok(false);
        }
        if let Some(regex) = schema.get("pattern") {
            if !pattern(regex)?.is_match(text) {
                return Ok(false);
            }
        }
        if schema.get("format").is_some()
            && (!text.is_ascii()
                || text.chars().any(|c| c.is_whitespace() || c.is_control())
                || url::Url::parse(text).is_err())
        {
            return Ok(false);
        }
    }
    if let Some(values) = value.as_object() {
        if !bounds(schema, "minProperties", "maxProperties", values.len()) {
            return Ok(false);
        }
        if let Some(required) = schema.get("required") {
            if array(required)?
                .iter()
                .filter_map(Value::as_str)
                .any(|key| !values.contains_key(key))
            {
                return Ok(false);
            }
        }
        let properties = schema.get("properties").and_then(Value::as_object);
        for (key, value) in values {
            let constraint = properties
                .and_then(|properties| properties.get(key))
                .or_else(|| schema.get("additionalProperties"));
            if let Some(constraint) = constraint {
                if !accepts(constraint, value, depth + 1)? {
                    return Ok(false);
                }
            }
        }
    }
    if let Some(values) = value.as_array() {
        if !bounds(schema, "minItems", "maxItems", values.len()) {
            return Ok(false);
        }
        if schema.get("uniqueItems") == Some(&Value::Bool(true))
            && values
                .iter()
                .enumerate()
                .any(|(index, value)| values[..index].contains(value))
        {
            return Ok(false);
        }
        let prefix = schema
            .get("prefixItems")
            .or_else(|| schema.get("items").filter(|items| items.is_array()))
            .and_then(Value::as_array);
        for (index, value) in values.iter().enumerate() {
            let constraint = prefix
                .and_then(|items| items.get(index))
                .or_else(|| schema.get("items").filter(|items| !items.is_array()));
            if let Some(constraint) = constraint {
                if !accepts(constraint, value, depth + 1)? {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}
