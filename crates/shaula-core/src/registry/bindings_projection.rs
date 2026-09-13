//! Schema-driven split of Template bindings into non-secret values and
//! presence-only markers (spec 0038). The artifact's
//! `schemas/bindings.schema.json` is the per-field `sensitive` authority:
//! an omitted annotation is conservatively sensitive, so an unreadable or
//! unknown schema can only fail closed to presence markers, never leak a
//! value. Secret bytes never enter a projection, a Debug output or an
//! identity string built here.

use std::collections::BTreeMap;

use crate::error::{CoreError, CoreResult, ReasonCode};
use serde_json::{Map, Value};

const MAX_DEPTH: usize = 16;

fn invalid(summary: &str) -> CoreError {
    CoreError::new(ReasonCode::TemplateInvalid, summary)
}

/// The per-field sensitivity and structural validation authority parsed
/// from one artifact's bindings schema document.
#[derive(Debug, Clone)]
pub struct BindingsSchema {
    /// `true` means the field's value is protected; unknown fields are
    /// always sensitive (fail closed).
    sensitive: BTreeMap<String, bool>,
    schema: Value,
}

impl BindingsSchema {
    /// Parses a bindings schema document. A non-object document or a
    /// malformed sensitivity annotation is an error, never a silent
    /// permissive default.
    pub fn parse(schema: &Value) -> CoreResult<Self> {
        if !schema.is_object() {
            return Err(invalid("bindings schema must be an object"));
        }
        let mut sensitive = BTreeMap::new();
        if let Some(fields) = schema.get("properties").and_then(Value::as_object) {
            for key in fields.keys() {
                sensitive.insert(
                    key.clone(),
                    inherited_sensitive(schema, key, 0)? || field_sensitive(schema, key, 0)?,
                );
            }
        }
        Ok(Self {
            sensitive,
            schema: schema.clone(),
        })
    }

    /// A schema that protects every stored field: the only safe view of
    /// bindings whose schema is absent or unreadable.
    pub fn all_sensitive() -> Self {
        Self {
            sensitive: BTreeMap::new(),
            schema: Value::Object(Map::new()),
        }
    }

    /// Whether the field is protected. Unknown fields are sensitive.
    pub fn sensitive(&self, field: &str) -> bool {
        self.sensitive.get(field).copied().unwrap_or(true)
    }

    /// Whether the schema declares the field at all.
    pub fn declares(&self, field: &str) -> bool {
        self.sensitive.contains_key(field)
    }

    /// The spec 0038 §2 read projection: non-sensitive values verbatim,
    /// sensitive fields as `{"sensitive": true, "set": <bool>}` presence
    /// markers — never a value, prefix, suffix, hash or length.
    pub fn project(&self, bindings: &Map<String, Value>) -> Map<String, Value> {
        bindings
            .iter()
            .map(|(key, value)| {
                let projected = if self.sensitive(key) {
                    serde_json::json!({"sensitive": true, "set": !value.is_null()})
                } else {
                    value.clone()
                };
                (key.clone(), projected)
            })
            .collect()
    }

    /// The spec 0038 §3 update merge: the submitted object is a complete
    /// desired binding set resolved per field against the base. A
    /// sensitive field omitted or submitted as the keep-sentinel `null`
    /// keeps the stored value; any other value replaces it. A
    /// non-sensitive field's submitted value replaces; omission keeps.
    /// The `null` sentinel is never stored as a value.
    pub fn merge_update(
        &self,
        base: &Map<String, Value>,
        submitted: &Map<String, Value>,
    ) -> CoreResult<Map<String, Value>> {
        for (key, value) in submitted {
            if !self.declares(key) {
                return Err(invalid(&format!("unknown template binding field {key}")));
            }
            if is_presence_marker(value) {
                return Err(invalid(
                    "template binding presence markers cannot be submitted",
                ));
            }
        }
        let mut merged = base.clone();
        for (key, value) in submitted {
            if self.sensitive(key) && value.is_null() {
                // Keep-sentinel: retain the stored value.
                continue;
            }
            merged.insert(key.clone(), value.clone());
        }
        Ok(merged)
    }

    /// Structural validation of a complete binding set against the
    /// schema before admission (spec 0038 §3): required presence, unknown
    /// fields under `additionalProperties: false` and a bounded subset of
    /// JSON Schema type checks. A missing required field or an invalid
    /// value is rejected without any partial write.
    pub fn validate(&self, bindings: &Map<String, Value>) -> CoreResult<()> {
        validate_node(&self.schema, &Value::Object(bindings.clone()), 0)
    }
}

/// The read-side presence marker shape (`{"sensitive": true, "set": …}`).
/// Submitting one back as a value is an echo mistake, never an update.
fn is_presence_marker(value: &Value) -> bool {
    let Some(fields) = value.as_object() else {
        return false;
    };
    fields.len() == 2
        && fields.get("sensitive") == Some(&Value::Bool(true))
        && fields.get("set").is_some_and(Value::is_boolean)
}

/// Sensitivity of one declared top-level field: its own annotation (and
/// any nested protected material) propagates up; the conservative
/// default for an omitted annotation is sensitive.
fn field_sensitive(schema: &Value, key: &str, depth: usize) -> CoreResult<bool> {
    if depth > MAX_DEPTH {
        return Err(invalid(
            "bindings sensitivity schema exceeds the depth limit",
        ));
    }
    let Some(field) = schema.get("properties").and_then(|p| p.get(key)) else {
        return Ok(true);
    };
    sensitive_of(field, depth)
}

/// Port of the shaula-template discovery sensitivity rule: an explicit
/// boolean annotation wins (with nested `true` protecting the whole
/// subtree); anything else is protected.
fn sensitive_of(node: &Value, depth: usize) -> CoreResult<bool> {
    if depth > MAX_DEPTH {
        return Err(invalid(
            "bindings sensitivity schema exceeds the depth limit",
        ));
    }
    let mut protected = match node.get("sensitive") {
        Some(Value::Bool(value)) => *value,
        None => true,
        _ => return Err(invalid("variable sensitive annotation must be boolean")),
    };
    if let Some(properties) = node.get("properties").and_then(Value::as_object) {
        for child in properties.values() {
            protected |= sensitive_of(child, depth + 1)?;
        }
    }
    for keyword in ["items", "additionalProperties", "not"] {
        if let Some(child) = node.get(keyword).filter(|c| c.is_object()) {
            protected |= sensitive_of(child, depth + 1)?;
        }
    }
    for keyword in ["items", "prefixItems", "anyOf", "allOf", "oneOf"] {
        if let Some(children) = node.get(keyword).and_then(Value::as_array) {
            for child in children {
                protected |= sensitive_of(child, depth + 1)?;
            }
        }
    }
    Ok(protected)
}

/// Combinator branches (`anyOf`/`allOf`/`oneOf`/`not`) can declare the
/// field's sensitivity on the parent's behalf.
fn inherited_sensitive(schema: &Value, key: &str, depth: usize) -> CoreResult<bool> {
    if depth > MAX_DEPTH {
        return Err(invalid(
            "bindings sensitivity schema exceeds the depth limit",
        ));
    }
    let mut protected = schema.get("sensitive") == Some(&Value::Bool(true));
    if let Some(field) = schema.get("properties").and_then(|fields| fields.get(key)) {
        protected |= sensitive_of(field, depth + 1)?;
    }
    for keyword in ["anyOf", "allOf", "oneOf"] {
        for branch in schema
            .get(keyword)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            protected |= inherited_sensitive(branch, key, depth + 1)?;
        }
    }
    if let Some(branch) = schema.get("not") {
        protected |= inherited_sensitive(branch, key, depth + 1)?;
    }
    Ok(protected)
}

fn type_matches(declared: &str, value: &Value) -> bool {
    match declared {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "number" => value.is_number(),
        "integer" => value.is_i64() || value.is_u64(),
        "null" => value.is_null(),
        _ => true,
    }
}

/// A bounded JSON Schema subset validator for merged binding sets. It is
/// deliberately strict: unsupported authority (`$ref`) fails closed
/// rather than degrading to "anything passes".
fn validate_node(schema: &Value, value: &Value, depth: usize) -> CoreResult<()> {
    if depth > MAX_DEPTH {
        return Err(invalid("bindings schema exceeds the depth limit"));
    }
    if schema.get("$ref").is_some() {
        return Err(invalid("bindings schema references are unsupported"));
    }
    if let Some(declared) = schema.get("type").and_then(Value::as_str) {
        if !type_matches(declared, value) {
            return Err(invalid("template binding value has the wrong type"));
        }
    }
    if let Some(types) = schema.get("type").and_then(Value::as_array) {
        let ok = types
            .iter()
            .filter_map(Value::as_str)
            .any(|declared| type_matches(declared, value));
        if !ok {
            return Err(invalid("template binding value has the wrong type"));
        }
    }
    match schema.get("enum").and_then(Value::as_array) {
        Some(options) if !options.is_empty() && !options.contains(value) => {
            return Err(invalid("template binding value is not an allowed option"));
        }
        _ => {}
    }
    if let Some(expected) = schema.get("const") {
        if expected != value {
            return Err(invalid(
                "template binding value does not match the constant",
            ));
        }
    }
    if let Some(text) = value.as_str() {
        if let Some(min) = schema.get("minLength").and_then(Value::as_u64) {
            if (text.len() as u64) < min {
                return Err(invalid("template binding value is too short"));
            }
        }
        if let Some(max) = schema.get("maxLength").and_then(Value::as_u64) {
            if (text.len() as u64) > max {
                return Err(invalid("template binding value is too long"));
            }
        }
        if let Some(pattern) = schema.get("pattern").and_then(Value::as_str) {
            let regex = regex::Regex::new(pattern)
                .map_err(|_| invalid("bindings schema pattern is invalid"))?;
            if !regex.is_match(text) {
                return Err(invalid("template binding value does not match the pattern"));
            }
        }
    }
    if let Some(number) = value.as_f64() {
        if let Some(min) = schema.get("minimum").and_then(Value::as_f64) {
            if number < min {
                return Err(invalid("template binding value is below the minimum"));
            }
        }
        if let Some(max) = schema.get("maximum").and_then(Value::as_f64) {
            if number > max {
                return Err(invalid("template binding value is above the maximum"));
            }
        }
    }
    if let Some(fields) = value.as_object() {
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for field in required.iter().filter_map(Value::as_str) {
                if !fields.contains_key(field) {
                    return Err(invalid(&format!(
                        "template binding is missing required field {field}"
                    )));
                }
            }
        }
        let properties = schema.get("properties").and_then(Value::as_object);
        match schema.get("additionalProperties") {
            Some(Value::Bool(false)) => {
                for key in fields.keys() {
                    if !properties.is_some_and(|p| p.contains_key(key)) {
                        return Err(invalid(&format!("unknown template binding field {key}")));
                    }
                }
            }
            Some(other) if other.is_object() => {
                for (key, child) in fields {
                    let rule = properties.and_then(|p| p.get(key)).unwrap_or(other);
                    validate_node(rule, child, depth + 1)?;
                }
            }
            _ => {}
        }
        if let Some(properties) = properties {
            for (key, child) in fields {
                if let Some(rule) = properties.get(key) {
                    validate_node(rule, child, depth + 1)?;
                }
            }
        }
    }
    if let Some(items) = value.as_array() {
        if let Some(rule) = schema.get("items").filter(|r| r.is_object()) {
            for child in items {
                validate_node(rule, child, depth + 1)?;
            }
        }
        if let Some(rules) = schema.get("prefixItems").and_then(Value::as_array) {
            for (rule, child) in rules.iter().zip(items.iter()) {
                validate_node(rule, child, depth + 1)?;
            }
        }
    }
    if let Some(branches) = schema.get("allOf").and_then(Value::as_array) {
        for branch in branches {
            validate_node(branch, value, depth + 1)?;
        }
    }
    for keyword in ["anyOf", "oneOf"] {
        let Some(branches) = schema.get(keyword).and_then(Value::as_array) else {
            continue;
        };
        let matched = branches
            .iter()
            .filter(|branch| validate_node(branch, value, depth + 1).is_ok())
            .count();
        if matched == 0 {
            return Err(invalid("template binding value matches no allowed variant"));
        }
        if keyword == "oneOf" && matched > 1 {
            return Err(invalid("template binding value matches multiple variants"));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "bindings_projection_tests.rs"]
mod tests;
