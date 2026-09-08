//! The shared parsed input authority for admission and visual projections.

use serde_json::{Map, Value};
use shaula_core::error::{CoreError, CoreResult, ReasonCode};

#[path = "service_input_schema.rs"]
mod schema;
pub(super) use schema::validate_value;

pub(super) struct InputAuthority {
    pub policy: Map<String, Value>,
    pub schema: Option<Value>,
}

impl InputAuthority {
    pub fn parse(policy_json: &str, schema_json: Option<&str>) -> CoreResult<Self> {
        let policy: Value = serde_json::from_str(policy_json)
            .map_err(|e| CoreError::new(ReasonCode::SpecInvalid, format!("policy invalid: {e}")))?;
        let Value::Object(policy) = policy else {
            return Err(CoreError::new(
                ReasonCode::SpecInvalid,
                "policy must be an object",
            ));
        };
        let schema = schema_json
            .map(|text| {
                let value: Value = serde_json::from_str(text).map_err(|e| {
                    CoreError::new(ReasonCode::SpecInvalid, format!("schema invalid: {e}"))
                })?;
                schema::schema_shape(&value)?;
                Ok::<_, CoreError>(value)
            })
            .transpose()?;
        Ok(Self { policy, schema })
    }

    /// Complete Fleet admission: neither a projection nor a default can bypass this.
    pub fn validate(&self, inputs: &Map<String, Value>) -> CoreResult<()> {
        if inputs.len() > 32 {
            return Err(CoreError::new(ReasonCode::SpecInvalid, "too many inputs"));
        }
        if let Some(schema) = &self.schema {
            validate_value("", &Value::Object(inputs.clone()), schema)?;
        }
        for (key, value) in inputs {
            let Some(allowed) = self.policy.get(key).and_then(Value::as_array) else {
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
}

pub(crate) fn validate_inputs(
    inputs: &Map<String, Value>,
    policy_json: &str,
    schema_json: Option<&str>,
) -> CoreResult<()> {
    InputAuthority::parse(policy_json, schema_json)?.validate(inputs)
}

#[cfg(test)]
#[path = "service_validation_tests.rs"]
mod tests;
