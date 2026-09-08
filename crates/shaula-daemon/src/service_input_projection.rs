//! Projects only policy-approved whole values through the shared admission authority.

use std::collections::BTreeSet;

use serde_json::{Map, Value};
use shaula_core::registry::{
    InputContractProjection, InputContractReadError, InputField, InputOption,
};

use super::super::validation::{validate_value, InputAuthority};

pub(super) const MAX_BYTES: usize = 1024 * 1024;
type ProjectionResult<T> = Result<T, InputContractReadError>;

/// Counts retained serialized values during construction, before an oversized graph accumulates.
#[derive(Default)]
struct Budget(usize);

impl std::io::Write for Budget {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        if self.0 > MAX_BYTES {
            return Err(std::io::Error::other("projection limit exceeded"));
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Budget {
    fn count(&mut self, value: &impl serde::Serialize) -> ProjectionResult<()> {
        serde_json::to_writer(self, value)
            .map_err(|_| unavailable("input contract exceeds the 1 MiB projection limit"))
    }
}

pub(super) fn unavailable(reason: impl Into<String>) -> InputContractReadError {
    InputContractReadError::Unavailable {
        reason: reason.into(),
    }
}

fn option(value: &Value) -> ProjectionResult<InputOption> {
    fn bounded_depth(value: &Value, depth: usize) -> bool {
        depth <= 16
            && match value {
                Value::Array(values) => values.iter().all(|v| bounded_depth(v, depth + 1)),
                Value::Object(values) => values.values().all(|v| bounded_depth(v, depth + 1)),
                _ => true,
            }
    }
    if !bounded_depth(value, 0) {
        return Err(unavailable(
            "approved value exceeds the display depth limit of 16",
        ));
    }
    let value_json = value.to_string();
    if value_json.len() > MAX_BYTES {
        return Err(unavailable(
            "input contract exceeds the 1 MiB projection limit",
        ));
    }
    Ok(InputOption { value_json })
}

fn add_option<'a>(
    values: &mut Vec<&'a Value>,
    options: &mut Vec<InputOption>,
    value: &'a Value,
    budget: &mut Budget,
) -> ProjectionResult<()> {
    if !values.contains(&value) {
        if options.len() == 256 {
            return Err(unavailable(
                "input contract exceeds the limit of 256 options",
            ));
        }
        let option = option(value)?;
        budget.count(&option)?;
        options.push(option);
        values.push(value);
    }
    Ok(())
}

fn required_unavailable(key: &str) -> InputContractReadError {
    let bounded: String = key.chars().take(80).collect();
    unavailable(format!(
        "required input {bounded:?} has no valid approved option"
    ))
}

fn readable_label(key: &str) -> String {
    let words = key.replace(['_', '-'], " ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => key.to_owned(),
    }
}

pub(super) fn project(policy: &str, schema: &str) -> ProjectionResult<InputContractProjection> {
    let mut budget = Budget::default();
    // Parser diagnostics can contain publisher-supplied text; only fixed bounded reasons leave here.
    let authority = InputAuthority::parse(policy, Some(schema))
        .map_err(|_| unavailable("parameter schema or input policy is invalid or unsupported"))?;
    if authority.policy.values().any(|value| !value.is_array()) {
        return Err(unavailable(
            "input policy entries must be arrays of approved values",
        ));
    }
    let root = authority
        .schema
        .as_ref()
        .ok_or_else(|| unavailable("parameter schema is absent"))?;
    if let Some(presets) = root.get("enum").and_then(Value::as_array) {
        let mut values = Vec::new();
        let mut options = Vec::new();
        for preset in presets {
            if preset
                .as_object()
                .is_some_and(|inputs| authority.validate(inputs).is_ok())
            {
                add_option(&mut values, &mut options, preset, &mut budget)?;
            }
        }
        if options.is_empty() {
            return Err(unavailable(
                "root enum has no valid approved input configuration",
            ));
        }
        return Ok(InputContractProjection::Presets { presets: options });
    }
    if root
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind != "object")
    {
        return Err(unavailable(
            "parameter schema does not allow an inputs object",
        ));
    }
    let required: BTreeSet<&str> = root
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if required.len() > 32 {
        return Err(unavailable(
            "required inputs exceed the admission limit of 32 keys",
        ));
    }
    let properties = root.get("properties").and_then(Value::as_object);
    let additional = root.get("additionalProperties").and_then(Value::as_bool) != Some(false);
    let mut fields = Vec::new();
    let mut witness = Map::new();
    for (key, candidates) in &authority.policy {
        let declared = properties.and_then(|props| props.get(key));
        if declared.is_none() && !additional {
            continue;
        }
        let mut values = Vec::new();
        let mut options = Vec::new();
        for value in candidates.as_array().into_iter().flatten() {
            if declared.is_none_or(|schema| validate_value(key, value, schema).is_ok()) {
                add_option(&mut values, &mut options, value, &mut budget)?;
            }
        }
        if options.is_empty() {
            continue;
        }
        if fields.len() == 256 {
            return Err(unavailable(
                "input contract exceeds the limit of 256 fields",
            ));
        }
        if required.contains(key.as_str()) {
            // This is solely a satisfiability witness, never an emitted/default input value.
            witness.insert(key.clone(), values[0].clone());
        }
        let label = declared
            .and_then(|schema| schema.get("title"))
            .and_then(Value::as_str)
            .filter(|title| !title.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                if declared.is_some() {
                    readable_label(key)
                } else {
                    key.clone()
                }
            });
        let description = declared
            .and_then(|schema| schema.get("description"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        budget.count(&key)?;
        budget.count(&label)?;
        budget.count(&description)?;
        fields.push(InputField {
            key: key.clone(),
            label,
            description,
            required: required.contains(key.as_str()),
            options,
        });
    }
    for key in required {
        if !witness.contains_key(key) {
            return Err(required_unavailable(key));
        }
    }
    authority
        .validate(&witness)
        .map_err(|_| unavailable("parameter schema has no valid approved input configuration"))?;
    fields.sort_unstable_by(|left, right| left.key.cmp(&right.key));
    Ok(InputContractProjection::Fields { fields })
}

#[cfg(test)]
#[path = "service_input_projection_tests.rs"]
mod tests;
