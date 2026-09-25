use super::*;
impl ControlPlane {
    /// Resolves one pool spec's members to their profiles' current Active
    /// revisions, validating each member's inputs against the pinned
    /// revision's alias policy and artifact parameter schema — the same
    /// two-authority gate the inline pool path enforces (spec 0037 §2).
    pub(crate) async fn resolve_pool_members(
        &self,
        spec: &TemplatePoolSpec,
    ) -> CoreResult<Result<Vec<ResolvedTemplatePoolMember>, MutationError>> {
        let mut resolved = Vec::with_capacity(spec.members.len());
        for member in &spec.members {
            let pin = match self
                .resolve_template_ref(&member.template_profile_ref)
                .await
            {
                Ok(pin) => pin,
                Err(e) => return Ok(Err(unprocessable(e.code, e.summary))),
            };
            let policy = self
                .store
                .template_revision_get(&pin.0, pin.1)
                .await?
                .and_then(|r| r.fleet_input_policy_json)
                .unwrap_or_else(|| "{}".into());
            let schema = self.store.artifact_parameter_schema(&pin.2).await?;
            if schema.trim().is_empty() {
                return Err(CoreError::new(
                    ReasonCode::StorageUnavailable,
                    "pool member artifact has a blank parameter schema document",
                ));
            }
            if let Err(e) =
                crate::service::validate_inputs(&member.template_inputs, &policy, Some(&schema))
            {
                return Ok(Err(unprocessable(e.code, e.summary)));
            }
            let inputs_digest = crate::service::template_inputs_digest(&member.template_inputs)?;
            resolved.push(ResolvedTemplatePoolMember {
                key: member.key.clone(),
                template_profile_key: pin.0,
                template_revision: pin.1,
                template_artifact_digest: pin.2,
                template_attestation_id: pin.3,
                template_inputs: member.template_inputs.clone(),
                inputs_digest,
                weight: member.weight,
                max_runners: member.max_runners,
            });
        }
        Ok(Ok(resolved))
    }
}
