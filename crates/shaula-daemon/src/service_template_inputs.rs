//! Exact-revision input materials, independent of pin selection and publication.

use serde_json::{Map, Value};
use shaula_core::error::{CoreError, CoreResult, ReasonCode};

use super::{super::ControlPlane, validate_inputs};

/// Retains the existing context-specific storage diagnostic.
pub(in crate::service) enum InputContext {
    Fleet,
    PoolMember,
}

/// Raw materials stay unparsed until the caller's preceding gates have run.
/// In particular, Fleet PUT checks its backend after reading the schema but
/// before classifying a blank schema or invalid inputs.
pub(in crate::service) struct TemplateInputs {
    policy: String,
    schema: String,
    context: InputContext,
}

impl TemplateInputs {
    /// The outer error is unavailable material; the inner error is rejected
    /// input. PUT and follow retain their own rejection/deferral policies.
    pub(in crate::service) fn validate(
        &self,
        inputs: &Map<String, Value>,
    ) -> CoreResult<CoreResult<()>> {
        if self.schema.trim().is_empty() {
            return Err(CoreError::new(
                ReasonCode::StorageUnavailable,
                match self.context {
                    InputContext::Fleet => "pinned artifact has a blank parameter schema document",
                    InputContext::PoolMember => {
                        "pool member artifact has a blank parameter schema document"
                    }
                },
            ));
        }
        Ok(validate_inputs(inputs, &self.policy, Some(&self.schema)))
    }
}

impl ControlPlane {
    /// Load only the selected exact pin: never resolve Active, choose a pin,
    /// check a Runner Backend, or publish a revision here.
    pub(in crate::service) async fn load_template_inputs(
        &self,
        pin: &(String, i64, String, String),
        context: InputContext,
    ) -> CoreResult<TemplateInputs> {
        let policy = self
            .store
            .template_revision_get(&pin.0, pin.1)
            .await?
            .and_then(|revision| revision.fleet_input_policy_json)
            .unwrap_or_else(|| "{}".into());
        let schema = self.store.artifact_parameter_schema(&pin.2).await?;
        Ok(TemplateInputs {
            policy,
            schema,
            context,
        })
    }
}
