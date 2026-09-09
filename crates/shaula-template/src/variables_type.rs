//! The supported Terraform type grammar and literal defaults, interpreted from HCL AST nodes.

use std::collections::BTreeMap;

use hcl::{Expression, ObjectKey};
use serde_json::Value;
use shaula_core::error::CoreResult;

use super::{invalid, MAX_DEPTH, MAX_FIELDS};

#[derive(Debug)]
pub(super) enum Kind {
    String,
    Number,
    Bool,
    Object(BTreeMap<String, Node>),
    List(Box<Node>),
    Map(Box<Node>),
    Tuple(Vec<Node>),
}

#[derive(Debug)]
pub(super) struct Node {
    pub kind: Kind,
    pub optional: bool,
    pub default: Option<Value>,
}

impl Kind {
    pub fn name(&self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Number => "number",
            Self::Bool => "boolean",
            Self::Object(_) | Self::Map(_) => "object",
            Self::List(_) | Self::Tuple(_) => "array",
        }
    }
}

fn call<'a>(expr: &'a Expression, name: &str) -> Option<&'a [Expression]> {
    let Expression::FuncCall(function) = expr else {
        return None;
    };
    (function.name.namespace.is_empty()
        && function.name.name.as_str() == name
        && !function.expand_final)
        .then_some(function.args.as_slice())
}

fn object(expr: &Expression) -> CoreResult<&hcl::Object<ObjectKey, Expression>> {
    match call(expr, "object") {
        Some([Expression::Object(fields)]) => Ok(fields),
        _ => Err(invalid(
            "variable discovery requires an explicit object type",
        )),
    }
}

fn key(key: &ObjectKey) -> CoreResult<&str> {
    match key {
        ObjectKey::Identifier(key) => Ok(key.as_str()),
        ObjectKey::Expression(Expression::String(key)) => Ok(key),
        _ => Err(invalid(
            "variable object keys must be literal identifiers or strings",
        )),
    }
}

/// Only bindings and parameters are publisher-defined; v2 adds system-owned setup_info.
pub(super) fn envelope(
    expr: &Expression,
    input_version: u32,
) -> CoreResult<BTreeMap<String, Node>> {
    let fields = object(expr)?;
    let mut nodes = BTreeMap::new();
    let mut names = std::collections::BTreeSet::new();
    for (field, expr) in fields {
        let field = key(field)?;
        if !names.insert(field) {
            return Err(invalid("shaula variable contains duplicate system members"));
        }
        let expected = match field {
            "contract_version" => Some("number"),
            "generation" => Some("any"),
            "setup_info" if input_version == 2 => Some("any"),
            "jit_config" | "bindings_digest" => Some("string"),
            "bindings" | "parameters" => None,
            _ => return Err(invalid("shaula variable declares an unknown system member")),
        };
        if let Some(expected) = expected {
            if !matches!(expr, Expression::Variable(value) if value.as_str() == expected) {
                return Err(invalid(
                    "shaula variable changes a protected system member type",
                ));
            }
        } else {
            let node = parse(expr, 0, false)?;
            if !matches!(node.kind, Kind::Object(_)) {
                return Err(invalid(
                    "bindings and parameters must declare explicit object types",
                ));
            }
            nodes.insert(field.to_owned(), node);
        }
    }
    if names.len() != if input_version == 2 { 7 } else { 6 } {
        return Err(invalid(
            "shaula variable is missing a system envelope member",
        ));
    }
    Ok(nodes)
}

fn parse(expr: &Expression, depth: usize, attribute: bool) -> CoreResult<Node> {
    if depth > MAX_DEPTH {
        return Err(invalid("variable type exceeds the depth limit"));
    }
    if let Some(args) = call(expr, "optional") {
        if !attribute || !(1..=2).contains(&args.len()) {
            return Err(invalid("optional is supported only on object attributes"));
        }
        let mut node = parse(&args[0], depth + 1, false)?;
        node.optional = true;
        if let Some(default) = args.get(1) {
            let default = literal(default, 0)?;
            if !default.is_null() && !matches_value(&node, &default) {
                return Err(invalid(
                    "Terraform default does not strictly match its declared type",
                ));
            }
            node.default = Some(default);
        }
        return Ok(node);
    }
    let kind = if let Expression::Variable(value) = expr {
        match value.as_str() {
            "string" => Kind::String,
            "number" => Kind::Number,
            "bool" => Kind::Bool,
            _ => return Err(invalid("variable type is unsupported for static discovery")),
        }
    } else if let Some(args) = call(expr, "object") {
        let [Expression::Object(fields)] = args else {
            return Err(invalid("object type must contain a literal attribute map"));
        };
        if fields.len() > MAX_FIELDS {
            return Err(invalid("variable object exceeds the field limit"));
        }
        let mut nodes = BTreeMap::new();
        for (field, value) in fields {
            let field = key(field)?;
            if nodes
                .insert(field.to_owned(), parse(value, depth + 1, true)?)
                .is_some()
            {
                return Err(invalid("variable object contains duplicate attributes"));
            }
        }
        Kind::Object(nodes)
    } else if let Some(args) = call(expr, "list") {
        let [element] = args else {
            return Err(invalid("list type requires one element type"));
        };
        Kind::List(Box::new(parse(element, depth + 1, false)?))
    } else if let Some(args) = call(expr, "map") {
        let [element] = args else {
            return Err(invalid("map type requires one element type"));
        };
        Kind::Map(Box::new(parse(element, depth + 1, false)?))
    } else if let Some(args) = call(expr, "tuple") {
        let [Expression::Array(elements)] = args else {
            return Err(invalid("tuple type requires a literal type list"));
        };
        if elements.len() > MAX_FIELDS {
            return Err(invalid("variable tuple exceeds the element limit"));
        }
        Kind::Tuple(
            elements
                .iter()
                .map(|item| parse(item, depth + 1, false))
                .collect::<CoreResult<_>>()?,
        )
    } else {
        return Err(invalid("variable type is unsupported for static discovery"));
    };
    Ok(Node {
        kind,
        optional: false,
        default: None,
    })
}

fn literal(expr: &Expression, depth: usize) -> CoreResult<Value> {
    if depth > MAX_DEPTH {
        return Err(invalid("variable default exceeds the depth limit"));
    }
    match expr {
        Expression::Null => Ok(Value::Null),
        Expression::Bool(value) => Ok(Value::Bool(*value)),
        Expression::Number(value) => serde_json::to_value(value)
            .map_err(|_| invalid("variable number cannot be represented as JSON")),
        Expression::String(value) => Ok(Value::String(value.clone())),
        Expression::Array(items) => items
            .iter()
            .map(|item| literal(item, depth + 1))
            .collect::<CoreResult<Vec<_>>>()
            .map(Value::Array),
        Expression::Object(fields) => {
            let mut values = serde_json::Map::new();
            for (field, value) in fields {
                if values
                    .insert(key(field)?.to_owned(), literal(value, depth + 1)?)
                    .is_some()
                {
                    return Err(invalid("variable default contains duplicate object keys"));
                }
            }
            Ok(Value::Object(values))
        }
        Expression::Parenthesis(inner) => literal(inner, depth + 1),
        _ => Err(invalid(
            "variable defaults must be literal values; expressions are not evaluated",
        )),
    }
}

/// Reject Terraform coercion or dropped object keys instead of describing a different value.
pub(super) fn matches_value(node: &Node, value: &Value) -> bool {
    // Terraform permits typed null collection/object members. Whether a caller
    // may explicitly supply them is still determined by the matching schema.
    if value.is_null() {
        return true;
    }
    match &node.kind {
        Kind::String => value.is_string(),
        Kind::Number => value.is_number(),
        Kind::Bool => value.is_boolean(),
        Kind::Object(fields) => value.as_object().is_some_and(|values| {
            values.iter().all(|(key, value)| {
                fields
                    .get(key)
                    .is_some_and(|node| matches_value(node, value))
            }) && fields
                .iter()
                .all(|(key, node)| node.optional || values.contains_key(key))
        }),
        Kind::List(element) => value
            .as_array()
            .is_some_and(|values| values.iter().all(|value| matches_value(element, value))),
        Kind::Map(element) => value
            .as_object()
            .is_some_and(|values| values.values().all(|value| matches_value(element, value))),
        Kind::Tuple(elements) => value.as_array().is_some_and(|values| {
            values.len() == elements.len()
                && elements
                    .iter()
                    .zip(values)
                    .all(|(node, value)| matches_value(node, value))
        }),
    }
}
