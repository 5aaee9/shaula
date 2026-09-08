//! Exact-revision input-contract reads; storage is never confused with an empty form.

use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::registry::{Actor, InputContractReadError, Scope, TemplateInputContract};

use super::ControlPlane;

#[path = "service_input_projection.rs"]
mod projection;

impl ControlPlane {
    pub(super) async fn input_contract_get_impl(
        &self,
        actor: &Actor,
        key: &str,
        revision: i64,
    ) -> CoreResult<Result<TemplateInputContract, InputContractReadError>> {
        if !actor.has(Scope::TemplateRead) {
            return Ok(Err(InputContractReadError::Forbidden));
        }
        let Some(head) = self.store.template_profile_get(key).await? else {
            return Ok(Err(InputContractReadError::NotFound));
        };
        let Some(row) = self.store.template_revision_get(key, revision).await? else {
            return Ok(Err(InputContractReadError::NotFound));
        };
        let schema = self
            .store
            .artifact_parameter_schema(&row.artifact_digest)
            .await?;
        if schema.trim().is_empty() {
            return Err(CoreError::new(
                ReasonCode::StorageUnavailable,
                "artifact parameter schema is blank",
            ));
        }
        let Some(policy) = row.fleet_input_policy_json else {
            return Ok(Err(projection::unavailable(
                "revision input policy is absent",
            )));
        };
        let projection = match projection::project(&policy, &schema) {
            Ok(value) => value,
            Err(error) => return Ok(Err(error)),
        };
        // Do not bind materials to a different incarnation if the key was recreated mid-read.
        if self
            .store
            .template_profile_get(key)
            .await?
            .is_none_or(|current| current.incarnation != head.incarnation)
        {
            return Ok(Err(projection::unavailable(
                "template identity changed during the read; retry",
            )));
        }
        let contract = TemplateInputContract {
            version: 1,
            profile_key: key.to_owned(),
            incarnation: head.incarnation,
            revision: row.revision,
            artifact_digest: row.artifact_digest,
            projection,
        };
        let encoded = serde_json::to_vec(&contract).map_err(|_| {
            CoreError::new(ReasonCode::Internal, "input contract could not be encoded")
        })?;
        if encoded.len() > projection::MAX_BYTES {
            return Ok(Err(projection::unavailable(
                "input contract exceeds the 1 MiB projection limit",
            )));
        }
        Ok(Ok(contract))
    }
}
