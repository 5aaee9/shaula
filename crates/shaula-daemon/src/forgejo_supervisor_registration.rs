//! At-most-once registration. The intent contains identity/authority, never a token.

use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::lifecycle::GenerationState;
use shaula_core::ports::forgejo::{ForgejoBootstrapMaterial, ForgejoRegistrationUncertainty};
use shaula_core::ports::EffectOutcome;
use shaula_core::registry::{GenerationRecord, OperationInsert};

use super::ForgejoPoolSupervisor;

impl ForgejoPoolSupervisor {
    pub(super) async fn register_generation(
        &self,
        generation: &GenerationRecord,
        now: i64,
    ) -> CoreResult<Option<ForgejoBootstrapMaterial>> {
        let _claim = self.gates.acquire_claim(&self.fleet_key).await;
        if self
            .current_head()
            .await?
            .is_none_or(|head| head.deletion_marker)
        {
            self.lifecycle
                .generation_advance(&generation.id, GenerationState::CleanupRequired, now)
                .await?;
            return Ok(None);
        }
        let operation_id = format!("forgejo-register-{}", generation.id);
        self.lifecycle
            .operation_insert(OperationInsert {
                id: operation_id.clone(),
                generation_id: generation.id.clone(),
                kind: "ForgejoRegistration".into(),
                state: "Starting".into(),
                provenance_json: Some(
                    serde_json::json!({
                        "fleet_key": self.fleet_key,
                        "fleet_revision": generation.fleet_revision,
                        "runner_name": generation.runner_name,
                        "instance_url": self.instance_url,
                        "auth": self.auth,
                        "labels": self.labels,
                        "ephemeral": true,
                    })
                    .to_string(),
                ),
                saved_plan_path: None,
                saved_plan_digest: None,
                now,
            })
            .await?;
        let registration = match self
            .forgejo
            .register_runner(&generation.runner_name, None)
            .await
        {
            Ok(EffectOutcome::Definite(registration)) => registration,
            Ok(EffectOutcome::Uncertain { .. }) => {
                self.classify_registration(generation, &operation_id, now)
                    .await?;
                return Ok(None);
            }
            Err(_) => {
                // Even a surprising adapter error must not turn into another POST.
                self.lifecycle
                    .operation_update_state(&operation_id, "Blocked", now)
                    .await?;
                self.lifecycle
                    .generation_advance(&generation.id, GenerationState::Quarantined, now)
                    .await?;
                return Ok(None);
            }
        };
        let runner_id = i64::try_from(registration.id).ok().filter(|id| *id > 0);
        let Some(runner_id) = runner_id else {
            self.lifecycle
                .generation_advance(&generation.id, GenerationState::Quarantined, now)
                .await?;
            return Err(CoreError::new(
                ReasonCode::CredentialMalformed,
                "invalid Forgejo runner ID",
            ));
        };
        // Persist remote identity BEFORE inspecting one-shot material or calling
        // the runtime: every subsequent failure retains cleanup evidence.
        if let Err(error) = self
            .lifecycle
            .generation_set_forgejo_runner(&generation.id, runner_id, &registration.uuid, now)
            .await
        {
            self.lifecycle
                .generation_advance(&generation.id, GenerationState::Quarantined, now)
                .await?;
            return Err(error);
        }
        self.lifecycle
            .operation_update_state(&operation_id, "Completed", now)
            .await?;
        match ForgejoBootstrapMaterial::new(
            self.instance_url.clone(),
            registration.uuid,
            registration.token,
            self.bootstrap_labels.clone(),
        ) {
            Ok(material) => Ok(Some(material)),
            Err(_) => {
                self.lifecycle
                    .generation_advance(&generation.id, GenerationState::CleanupRequired, now)
                    .await?;
                Ok(None)
            }
        }
    }

    /// This generation has never entered the template runtime, so any exact
    /// candidate is cleanup-only; a recovered token is neither assumed nor minted.
    pub(super) async fn classify_registration(
        &self,
        generation: &GenerationRecord,
        operation_id: &str,
        now: i64,
    ) -> CoreResult<()> {
        let classification = self
            .forgejo
            .classify_uncertain_registration(&generation.runner_name, &self.labels)
            .await;
        let next = match classification {
            Ok(ForgejoRegistrationUncertainty::None) => GenerationState::CleanupRequired,
            Ok(ForgejoRegistrationUncertainty::ExactlyOneCleanupRequired) => {
                match self
                    .forgejo
                    .uncertain_registration_candidates(&generation.runner_name, &self.labels)
                    .await
                {
                    Ok(candidates) if candidates.len() == 1 => {
                        let candidate = &candidates[0];
                        if let Ok(id) = i64::try_from(candidate.id) {
                            self.lifecycle
                                .generation_set_forgejo_runner(
                                    &generation.id,
                                    id,
                                    &candidate.uuid,
                                    now,
                                )
                                .await?;
                            GenerationState::CleanupRequired
                        } else {
                            GenerationState::Quarantined
                        }
                    }
                    _ => GenerationState::Quarantined,
                }
            }
            _ => GenerationState::Quarantined,
        };
        let phase = if next == GenerationState::Quarantined {
            "Blocked"
        } else {
            "Completed"
        };
        self.lifecycle
            .operation_update_state(operation_id, phase, now)
            .await?;
        self.lifecycle
            .generation_advance(&generation.id, next, now)
            .await
    }
}
