//! At-most-once registration. The intent contains identity/authority, never a token.

use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::gitlab::{ownership_note, GitlabRegistrationRole};
use shaula_core::lifecycle::GenerationState;
use shaula_core::ports::gitlab::{GitlabBootstrapMaterial, GitlabRunnerSpec};
use shaula_core::ports::EffectOutcome;
use shaula_core::registry::{GenerationRecord, OperationInsert};

use super::GitlabPoolSupervisor;
use crate::runner_operation::gitlab::is_owned_generation;

impl GitlabPoolSupervisor {
    pub(super) async fn register_generation(
        &self,
        generation: &GenerationRecord,
        now: i64,
    ) -> CoreResult<Option<GitlabBootstrapMaterial>> {
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
        let operation_id = format!("gitlab-register-{}", generation.id);
        self.lifecycle
            .operation_insert(OperationInsert {
                id: operation_id.clone(),
                generation_id: generation.id.clone(),
                kind: "GitlabRegistration".into(),
                state: "Starting".into(),
                provenance_json: Some(
                    serde_json::json!({
                        "fleet_key": self.fleet_key,
                        "fleet_revision": generation.fleet_revision,
                        "runner_name": generation.runner_name,
                        "instance_url": self.instance_url,
                        "auth": self.auth,
                        "tags": self.matching.tags,
                        "run_untagged": self.matching.run_untagged,
                    })
                    .to_string(),
                ),
                saved_plan_path: None,
                saved_plan_digest: None,
                now,
            })
            .await?;
        let spec = GitlabRunnerSpec {
            description: generation.runner_name.clone(),
            maintenance_note: ownership_note(
                &self.fleet_key,
                GitlabRegistrationRole::Generation,
                &generation.runner_name,
            ),
            role: GitlabRegistrationRole::Generation,
            matching: self.matching.clone(),
        };
        let registration = match self.gitlab.register_runner(&spec).await {
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
        let Some(runner_id) = i64::try_from(registration.id).ok().filter(|id| *id > 0) else {
            self.lifecycle
                .generation_advance(&generation.id, GenerationState::Quarantined, now)
                .await?;
            return Err(CoreError::new(
                ReasonCode::CredentialMalformed,
                "invalid GitLab runner ID",
            ));
        };
        // Persist remote identity BEFORE inspecting one-shot material or calling
        // the runtime: every subsequent failure retains cleanup evidence.
        if let Err(error) = self
            .lifecycle
            .generation_set_gitlab_runner(&generation.id, runner_id, now)
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
        match GitlabBootstrapMaterial::new(
            self.instance_url.clone(),
            registration.id,
            generation.runner_name.clone(),
            registration.token,
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

    /// This generation never entered the template runtime, so any exact
    /// candidate is cleanup-only; a token is neither recovered nor minted.
    pub(super) async fn classify_registration(
        &self,
        generation: &GenerationRecord,
        operation_id: &str,
        now: i64,
    ) -> CoreResult<()> {
        let next = match self.gitlab.list_runners(&generation.runner_name).await {
            Ok(runners) => {
                let named: Vec<_> = runners
                    .iter()
                    .filter(|r| r.description == generation.runner_name)
                    .collect();
                match named.as_slice() {
                    [] => GenerationState::CleanupRequired,
                    [candidate] if is_owned_generation(candidate, &self.fleet_key, generation) => {
                        match i64::try_from(candidate.id) {
                            Ok(id) => {
                                self.lifecycle
                                    .generation_set_gitlab_runner(&generation.id, id, now)
                                    .await?;
                                GenerationState::CleanupRequired
                            }
                            Err(_) => GenerationState::Quarantined,
                        }
                    }
                    _ => GenerationState::Quarantined,
                }
            }
            Err(_) => GenerationState::Quarantined,
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
