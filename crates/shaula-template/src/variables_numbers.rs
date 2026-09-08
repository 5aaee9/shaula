//! Refuse numeric rounding or overflow before discarding the preserving HCL representation.

use std::collections::BTreeMap;
use std::str::FromStr;

use bigdecimal::BigDecimal;
use hcl::edit::expr::{Expression, UnaryOp, UnaryOperator};
use hcl::edit::visit::{self, Visit};
use hcl::edit::{Formatted, Number};
use serde_json::{value::RawValue, Value};
use shaula_core::error::CoreResult;

use super::invalid;

#[derive(Default)]
struct NumberCheck {
    invalid: bool,
    depth: usize,
}

impl Visit for NumberCheck {
    fn visit_expr(&mut self, expression: &Expression) {
        if self.invalid {
            return;
        }
        self.depth += 1;
        if self.depth > 128 {
            self.invalid = true;
        } else {
            visit::visit_expr(self, expression);
        }
        self.depth -= 1;
    }

    fn visit_number(&mut self, number: &Formatted<Number>) {
        let encoded = serde_json::to_string(number.value()).ok();
        self.invalid |= !number
            .as_repr()
            .zip(encoded.as_deref())
            .is_some_and(|(original, encoded)| equivalent(original, encoded));
    }

    fn visit_unary_op(&mut self, unary: &UnaryOp) {
        // hcl-rs folds unary minus during conversion. Reject integer overflow and
        // nonliteral operators before that conversion can wrap or panic.
        if *unary.operator.value() != UnaryOperator::Neg {
            self.invalid = true;
            return;
        }
        let Expression::Number(number) = &unary.expr else {
            self.invalid = true;
            return;
        };
        if number
            .value()
            .as_u64()
            .is_some_and(|value| value > i64::MIN.unsigned_abs())
        {
            self.invalid = true;
        }
        self.visit_number(number);
    }
}

fn equivalent(original: &str, encoded: &str) -> bool {
    let original = (original.len() <= 1024)
        .then(|| BigDecimal::from_str(original).ok())
        .flatten();
    original.is_some() && original == BigDecimal::from_str(encoded).ok()
}

/// Check raw JSON tokens before rounded values can become public defaults or options.
pub(super) fn schema_json(text: &str) -> CoreResult<Value> {
    let value: Value = serde_json::from_str(text)
        .map_err(|_| invalid("template variable schema is invalid JSON"))?;
    verify_json(text, &value, 0)?;
    Ok(value)
}

fn verify_json(text: &str, value: &Value, depth: usize) -> CoreResult<()> {
    if depth > 64 {
        return Err(invalid("template variable schema exceeds the depth limit"));
    }
    let invalid_json = |_| invalid("template variable schema is invalid JSON");
    match value {
        Value::Number(number) if !equivalent(text, &number.to_string()) => {
            return Err(invalid(
                "schema number cannot be represented exactly as JSON",
            ));
        }
        Value::Array(items) => {
            let originals: Vec<&RawValue> = serde_json::from_str(text).map_err(invalid_json)?;
            for (original, item) in originals.iter().zip(items) {
                verify_json(original.get(), item, depth + 1)?;
            }
        }
        Value::Object(fields) => {
            let originals: BTreeMap<String, &RawValue> =
                serde_json::from_str(text).map_err(invalid_json)?;
            for (key, original) in originals {
                verify_json(original.get(), &fields[&key], depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn verify(expression: &Expression) -> CoreResult<()> {
    let mut check = NumberCheck::default();
    check.visit_expr(expression);
    if check.invalid {
        Err(invalid(
            "Terraform numeric default cannot be represented exactly as JSON",
        ))
    } else {
        Ok(())
    }
}
