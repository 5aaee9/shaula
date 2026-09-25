use super::*;
impl ControlPlane {
    pub(super) async fn fleet_get_read(
        &self,
        _actor: &Actor,
        key: &str,
    ) -> CoreResult<Result<FleetResource, MutationError>> {
        let Some(fleet) = self.store.fleet_get(key).await? else {
            return Ok(Err(MutationError::NotFound));
        };
        if fleet.tombstone {
            return Ok(Err(MutationError::Gone {
                tombstone: key.to_string(),
            }));
        }
        let Some(revision) = self.store.fleet_revision_latest(key).await? else {
            return Ok(Err(MutationError::NotFound));
        };
        let spec: FleetSpec = serde_json::from_str(&revision.spec_json)
            .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?;
        Ok(Ok(FleetResource {
            key: key.to_string(),
            spec,
            incarnation: fleet.incarnation,
            revision: fleet.desired_revision,
            resolved_template: revision.template_profile_key.clone().map(|k| {
                (
                    k,
                    revision.template_revision.unwrap_or_default(),
                    revision
                        .template_artifact_digest
                        .clone()
                        .unwrap_or_default(),
                    revision.template_attestation_id.clone().unwrap_or_default(),
                )
            }),
            resolved_template_pool: revision.template_pool.clone(),
            resolved_template_pool_ref: revision.template_pool_ref.clone(),
            resolved_auth: revision.auth_desired.clone(),
            created_at: revision.created_at,
            updated_at: revision.created_at,
        }))
    }
}
