//! Exact Template Revision and authentication material for Fleet admission.

use super::{unprocessable, AuthRevisionRef, ControlPlane};
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::fleet::FleetSpec;
use shaula_core::registry::MutationError;

impl ControlPlane {
    /// Resolves everything fleet PUT admission needs from the two
    /// authorities: the retained-or-resolved exact template pin, and the
    /// resolved active auth revision. Inputs are validated against the
    /// pinned revision's parameter schema AND its finite alias policy
    /// (spec 0002 section 4.1, 0005 section 5.1).
    pub(crate) async fn resolve_admission_materials(
        &self,
        key: &str,
        spec: &FleetSpec,
    ) -> CoreResult<
        Result<
            (
                Option<(String, i64, String, String)>,
                Vec<shaula_core::template_pool::ResolvedTemplatePoolMember>,
                Option<shaula_core::template_pool::FleetPoolRef>,
                AuthRevisionRef,
            ),
            MutationError,
        >,
    > {
        let previous_row = self.store.fleet_revision_latest(key).await?;
        let previous_pin: Option<(String, i64, String, String)> =
            previous_row.as_ref().and_then(|r| {
                let (k, rev) = (r.template_profile_key.clone()?, r.template_revision?);
                Some((
                    k,
                    rev,
                    r.template_artifact_digest.clone()?,
                    r.template_attestation_id.clone()?,
                ))
            });
        let previous_spec: Option<FleetSpec> = previous_row
            .as_ref()
            .and_then(|prev| serde_json::from_str(&prev.spec_json).ok());
        let reference_unchanged = previous_spec
            .as_ref()
            .is_some_and(|ps| super::template_referenced(ps) == super::template_referenced(spec));
        // Pool members carry immutable template pins. Reasserting an
        // unchanged pool must retain those pins; resolving Active here would
        // silently upgrade one member during an otherwise identical PUT.
        let pool_unchanged = previous_spec
            .as_ref()
            .is_some_and(|ps| ps.template_pool == spec.template_pool);
        // The retained pin preserves "PUT is not an implicit upgrade
        // channel" for an identical re-assertion (spec 0023 §2). Once the
        // spec's template inputs change, the new revision is admitted
        // against the fleet's actual follow-latest target — the profile's
        // current Active revision — whose parameter schema may have been
        // extended by a template update since the pin was minted. Inputs
        // changes already pass the zero-occupancy gate, so resolving the
        // pin to Active here is not an upgrade around the drain rule.
        let inputs_unchanged = previous_spec
            .as_ref()
            .is_some_and(|ps| ps.template_inputs == spec.template_inputs);
        // Spec 0037 §4: a shared-pool fleet freezes the pool's CURRENT
        // revision on this fleet revision; member rows live on the pool and
        // their input validation happened at pool admission. Catch-up to a
        // newer pool revision is the cascade's deferred job, not PUT's.
        let pool_ref = if let Some(pool_key) = spec
            .template_pool_ref
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            let Some(head) = self.store.template_pool_get(pool_key).await? else {
                return Ok(Err(unprocessable(
                    ReasonCode::TargetHiddenOrNotFound,
                    "template pool not found",
                )));
            };
            if head.tombstone || head.deletion_marker {
                return Ok(Err(unprocessable(
                    ReasonCode::SpecInvalid,
                    "template pool is deleted",
                )));
            }
            let Some(latest) = self.store.template_pool_revision_latest(pool_key).await? else {
                return Ok(Err(unprocessable(
                    ReasonCode::SpecInvalid,
                    "template pool has no revision",
                )));
            };
            if latest.members.is_empty() {
                return Ok(Err(unprocessable(
                    ReasonCode::SpecInvalid,
                    "template pool revision has no members",
                )));
            }
            // Spec 0037 §5: catching up to a newer revision of the SAME
            // pool is the cascade's deferred job, not PUT's. Re-freezing
            // to the latest revision here would read every capacity-only
            // update as a routing change and trip the zero-occupancy gate
            // (R9-04). Retain the fleet's frozen (key, revision) when the
            // key is unchanged; only an actual pool switch re-freezes.
            let frozen = previous_row
                .as_ref()
                .and_then(|row| row.template_pool_ref.clone());
            match frozen {
                Some((frozen_key, frozen_revision)) if frozen_key == pool_key => {
                    Some((frozen_key, frozen_revision))
                }
                _ => {
                    if let Err(error) = self.validate_pool_backends(&latest.members, spec).await {
                        return Ok(Err(unprocessable(error.code, error.summary)));
                    }
                    Some((pool_key.to_string(), latest.revision))
                }
            }
        } else {
            None
        };
        let template = if spec.template_pool.is_some() || pool_ref.is_some() {
            None
        } else if reference_unchanged && inputs_unchanged {
            previous_pin.clone()
        } else {
            match self.resolve_template_ref(&spec.template_profile_ref).await {
                Ok(found) => Some(found),
                Err(e) => return Ok(Err(unprocessable(e.code, e.summary))),
            }
        };
        if let Some((pin_key, pin_rev, pin_artifact, _)) = &template {
            let policy = self
                .store
                .template_revision_get(pin_key, *pin_rev)
                .await?
                .and_then(|r| r.fleet_input_policy_json)
                .unwrap_or_else(|| "{}".into());
            // Layer 1 authority: the artifact's declared parameter schema
            // (required/type/bounds) from the exact pinned artifact. A
            // read failure is an Err (`?` → 500): admission never
            // degrades a corrupt artifact to "no schema" (F08). A blank
            // document is equally corrupt — an empty schema is `{}`, not
            // whitespace (R5-04).
            let schema = self.store.artifact_parameter_schema(pin_artifact).await?;
            if let Err(error) = self
                .validate_template_backend(
                    pin_key,
                    *pin_rev,
                    pin_artifact,
                    spec,
                    &spec.template_inputs,
                )
                .await
            {
                return Ok(Err(unprocessable(error.code, error.summary)));
            }
            if schema.trim().is_empty() {
                return Err(CoreError::new(
                    ReasonCode::StorageUnavailable,
                    "pinned artifact has a blank parameter schema document",
                ));
            }
            // A rejected input is a CLIENT error: classified 422, never a
            // 500 (spec 0002 section 5.1 invalid-spec contract).
            if let Err(e) = super::validate_inputs(&spec.template_inputs, &policy, Some(&schema)) {
                return Ok(Err(unprocessable(e.code, e.summary)));
            }
        }
        // Shared routing owns its pins on the pool revision, not duplicated
        // inline rows. A capacity-only PUT must not turn a hydrated shared
        // snapshot into new inline pins that must still be current Active.
        let mut resolved_pool = if spec.template_pool.is_some() && pool_unchanged {
            previous_row
                .as_ref()
                .map(|row| row.template_pool.clone())
                .filter(|members| !members.is_empty())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        if resolved_pool.is_empty() {
            if let Some(pool) = &spec.template_pool {
                for member in &pool.members {
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
                        super::validate_inputs(&member.template_inputs, &policy, Some(&schema))
                    {
                        return Ok(Err(unprocessable(e.code, e.summary)));
                    }
                    let inputs_digest = super::template_inputs_digest(&member.template_inputs)?;
                    resolved_pool.push(shaula_core::template_pool::ResolvedTemplatePoolMember {
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
            }
        }
        if let Err(error) = self.validate_pool_backends(&resolved_pool, spec).await {
            return Ok(Err(unprocessable(error.code, error.summary)));
        }
        let auth_profile_ref = spec.auth_profile_ref();
        if let Err(e) = self
            .assert_auth_target_allowed(auth_profile_ref, spec)
            .await
        {
            return Ok(Err(unprocessable(e.code, e.summary)));
        }
        let resolved_auth = match self.resolve_auth_ref(auth_profile_ref).await {
            Ok(found) => found,
            Err(e) => return Ok(Err(unprocessable(e.code, e.summary))),
        };
        Ok(Ok((template, resolved_pool, pool_ref, resolved_auth)))
    }
}
