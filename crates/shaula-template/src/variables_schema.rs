//! Check schema agreement and attach its display/approval annotations to Terraform declarations.

use std::collections::BTreeSet;

use serde_json::Value;
use shaula_core::error::CoreResult;
use shaula_core::registry::{InputOption, TemplateVariable};

use super::constraints::accepts as schema_accepts;
use super::types::{matches_value, Kind, Node};
use super::{invalid, MAX_BYTES, MAX_DEPTH, MAX_FIELDS};

fn shape(node: &Node, schema: &Value, depth: usize) -> CoreResult<()> {
    if depth > MAX_DEPTH || !schema.is_object() {
        return Err(invalid("variable schema shape is invalid or too deep"));
    }
    let declared = schema.get("type").and_then(Value::as_str);
    if declared != Some(node.kind.name())
        && !(declared == Some("integer") && matches!(node.kind, Kind::Number))
    {
        return Err(invalid("Terraform variable type disagrees with its schema"));
    }
    if let Kind::Object(fields) = &node.kind {
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .ok_or_else(|| invalid("object variable schema must declare its properties"))?;
        if properties.len() != fields.len()
            || fields.keys().any(|key| !properties.contains_key(key))
        {
            return Err(invalid(
                "Terraform variable names disagree with schema properties",
            ));
        }
        let required = required(schema)?;
        if required.iter().any(|key| !fields.contains_key(*key)) {
            return Err(invalid("variable schema requires an undeclared property"));
        }
        for (key, node) in fields {
            let schema = &properties[key];
            let required = required.contains(key.as_str());
            let has_default = node.default.as_ref().is_some_and(|value| !value.is_null());
            if (!required && !node.optional) || (required && node.optional && !has_default) {
                return Err(invalid(
                    "Terraform variable optionality disagrees with its schema",
                ));
            }
            shape(node, schema, depth + 1)?;
        }
    }
    match &node.kind {
        Kind::List(element) => {
            if let Some(items) = schema.get("items").filter(|items| items.is_object()) {
                shape(element, items, depth + 1)?;
            }
            if let Some(prefix) = schema
                .get("prefixItems")
                .or_else(|| schema.get("items").filter(|items| items.is_array()))
                .and_then(Value::as_array)
            {
                for item in prefix {
                    shape(element, item, depth + 1)?;
                }
            }
        }
        Kind::Tuple(elements) => {
            if let Some(prefix) = schema
                .get("prefixItems")
                .or_else(|| schema.get("items").filter(|items| items.is_array()))
                .and_then(Value::as_array)
            {
                if prefix.len() != elements.len() {
                    return Err(invalid("Terraform tuple length disagrees with its schema"));
                }
                for (element, item) in elements.iter().zip(prefix) {
                    shape(element, item, depth + 1)?;
                }
            } else if let Some(items) = schema.get("items").filter(|items| items.is_object()) {
                for element in elements {
                    shape(element, items, depth + 1)?;
                }
            }
        }
        Kind::Map(element) => {
            if let Some(items) = schema
                .get("additionalProperties")
                .filter(|items| items.is_object())
            {
                shape(element, items, depth + 1)?;
            }
            if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
                for item in properties.values() {
                    shape(element, item, depth + 1)?;
                }
            }
        }
        _ => {}
    }
    if let Some(default) = schema.get("default") {
        if node.default.as_ref() != Some(default) {
            return Err(invalid(
                "schema default disagrees with the Terraform declaration",
            ));
        }
    }
    if let Some(default) = &node.default {
        // Terraform uses null for an omitted optional attribute. This is a declared
        // runtime default, not permission to submit null through the Fleet schema.
        if !default.is_null() && !schema_accepts(schema, default, depth)? {
            return Err(invalid("Terraform default is outside the variable schema"));
        }
    }
    if let Some(options) = schema.get("enum") {
        let options = options
            .as_array()
            .ok_or_else(|| invalid("variable schema enum must be an array"))?;
        if options.len() > MAX_FIELDS {
            return Err(invalid("variable schema exceeds the option limit"));
        }
        for option in options {
            if !matches_value(node, option) || !schema_accepts(schema, option, depth)? {
                return Err(invalid(
                    "schema option disagrees with the Terraform variable type",
                ));
            }
        }
    }
    Ok(())
}

fn required(schema: &Value) -> CoreResult<BTreeSet<&str>> {
    let Some(value) = schema.get("required") else {
        return Ok(BTreeSet::new());
    };
    let fields = value
        .as_array()
        .ok_or_else(|| invalid("variable schema required must be an array"))?;
    let mut result = BTreeSet::new();
    for field in fields {
        let field = field
            .as_str()
            .ok_or_else(|| invalid("variable schema required names must be strings"))?;
        if !result.insert(field) {
            return Err(invalid("variable schema required contains duplicate names"));
        }
    }
    Ok(result)
}

fn sensitive(schema: &Value, binding: bool, depth: usize) -> CoreResult<bool> {
    if depth > MAX_DEPTH {
        return Err(invalid(
            "variable sensitivity schema exceeds the depth limit",
        ));
    }
    let own = match schema.get("sensitive") {
        Some(Value::Bool(value)) => *value,
        None => binding,
        _ => return Err(invalid("variable sensitive annotation must be boolean")),
    };
    let mut protected = own;
    for property in schema
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|v| v.values())
    {
        protected |= sensitive(property, false, depth + 1)?;
    }
    for keyword in ["items", "additionalProperties", "not"] {
        if let Some(child) = schema.get(keyword).filter(|child| child.is_object()) {
            protected |= sensitive(child, false, depth + 1)?;
        }
    }
    for keyword in ["items", "prefixItems", "anyOf", "allOf", "oneOf"] {
        if let Some(children) = schema.get(keyword).and_then(Value::as_array) {
            for child in children {
                protected |= sensitive(child, false, depth + 1)?;
            }
        }
    }
    Ok(protected)
}

fn inherited_sensitive(schema: &Value, key: &str, depth: usize) -> CoreResult<bool> {
    if depth > MAX_DEPTH {
        return Err(invalid(
            "variable sensitivity schema exceeds the depth limit",
        ));
    }
    let mut protected = schema.get("sensitive") == Some(&Value::Bool(true));
    // Only the selected property's metadata applies; a sibling secret must not
    // make every other field secret merely because both share this group.
    if let Some(field) = schema.get("properties").and_then(|fields| fields.get(key)) {
        protected |= sensitive(field, false, depth + 1)?;
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

fn text(schema: &Value, key: &str) -> CoreResult<String> {
    match schema.get(key) {
        Some(Value::String(value)) if value.len() <= 8192 => Ok(value.clone()),
        None => Ok(String::new()),
        _ => Err(invalid(
            "variable display annotation must be a bounded string",
        )),
    }
}

fn json(value: &Value, depth: usize) -> CoreResult<String> {
    fn bounded(value: &Value, depth: usize) -> bool {
        depth <= MAX_DEPTH
            && match value {
                Value::Array(items) => items.iter().all(|item| bounded(item, depth + 1)),
                Value::Object(fields) => fields.values().all(|item| bounded(item, depth + 1)),
                _ => true,
            }
    }
    if !bounded(value, depth) {
        return Err(invalid("variable value exceeds the depth limit"));
    }
    let value = value.to_string();
    if value.len() > MAX_BYTES {
        return Err(invalid("variable value exceeds the 1 MiB limit"));
    }
    Ok(value)
}

pub(super) fn project(
    node: &Node,
    json_text: &str,
    binding: bool,
) -> CoreResult<Vec<TemplateVariable>> {
    let schema = super::numbers::schema_json(json_text)?;
    super::constraints::check(&schema, 0)?;
    shape(node, &schema, 0)?;
    let Kind::Object(fields) = &node.kind else {
        return Err(invalid("variables require a declared object type"));
    };
    let required = required(&schema)?;
    let mut result = Vec::new();
    for (key, node) in fields {
        let definition = &schema["properties"][key];
        let sensitive = inherited_sensitive(&schema, key, 0)? || sensitive(definition, binding, 0)?;
        let mut label = text(definition, "title")?;
        if label.trim().is_empty() {
            let words = key.replace(['_', '-'], " ");
            let mut chars = words.chars();
            label = chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default();
        }
        let default_value_json = if sensitive {
            None
        } else {
            node.default
                .as_ref()
                .map(|value| json(value, 0))
                .transpose()?
        };
        let mut options = Vec::new();
        if !sensitive {
            for option in definition
                .get("enum")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let option = InputOption {
                    value_json: json(option, 0)?,
                };
                if !options.contains(&option) {
                    options.push(option);
                }
            }
        }
        result.push(TemplateVariable {
            key: key.clone(),
            label,
            description: text(definition, "description")?,
            type_name: node.kind.name().to_owned(),
            required: required.contains(key.as_str()),
            sensitive,
            default_value_json,
            options,
        });
    }
    Ok(result)
}
