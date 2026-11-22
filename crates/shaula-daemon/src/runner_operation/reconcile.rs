//! Failed-Create reconciliation (ARD-0044): retries Phase 1 whole-shape anchor
//! once the worker is fenced, and for manifests declaring `reconcile`, runs
//! read-only remote `discover` plus targeted `import` before proceeding to
//! ordinary Destroy or empty completion.

use shaula_core::diagnostics::*;
use shaula_core::error::CoreResult;
use shaula_core::lifecycle::GenerationState as G;
use shaula_core::ports::{ReconcileClassification, TemplateOutcomeError, TemplateReconcileRequest};
use shaula_core::registry::GenerationRecord;
use shaula_core::template::{BindingsDigest, ProfileManifest};
use shaula_observability::MetricResult;

use super::destroy::DestroyProof;
use super::{anchor_failed_create, DestroyOutcome, RunnerOperation, RunnerRegistrations};
use crate::iac_metrics::record_iac;

pub(super) enum ReconcileStep {
    ReadyForDestroy(Box<DestroyProof>),
    Settled(DestroyOutcome),
}

impl<R: RunnerRegistrations> RunnerOperation<'_, R> {
    /// Resolves `DestroyProof` for a started Create: uses an existing retained
    /// identity, retries Phase 1 anchor once fenced, or runs optional read-only
    /// `reconcile` (ARD-0044) before falling back to Quarantine.
    pub(super) async fn resolve_destroy_or_reconcile(
        &self,
        generation: &GenerationRecord,
        entered_destroying_at: i64,
        now: i64,
        diagnostic: &mut Capture,
    ) -> CoreResult<ReconcileStep> {
        let id = generation.id.as_str();
        let Some(original_provenance) = self.lifecycle.operation_original_provenance(id).await?
        else {
            diagnostic.reason(Code::CleanupQuarantined, StageId::Completion, true);
            return Ok(ReconcileStep::Settled(
                self.quarantine(id, now, "Create provenance missing for destroy")
                    .await?,
            ));
        };
        let Some((_, bindings_digest)) = self
            .store
            .template_protected_bindings(
                &generation.template_profile_key,
                generation.template_revision,
            )
            .await?
        else {
            diagnostic.reason(Code::CleanupQuarantined, StageId::Completion, true);
            return Ok(ReconcileStep::Settled(
                self.quarantine(id, now, "admitted bindings missing for destroy")
                    .await?,
            ));
        };
        self.store
            .ensure_artifact_cached(&generation.template_artifact_digest)
            .await?;
        let Some(artifact_dir) = shaula_core::artifact_layout::artifact_dir(
            self.artifact_root,
            &generation.template_artifact_digest,
        ) else {
            diagnostic.reason(Code::CleanupQuarantined, StageId::Completion, true);
            return Ok(ReconcileStep::Settled(
                self.quarantine(id, now, "artifact digest malformed for destroy")
                    .await?,
            ));
        };
        let manifest = tokio::fs::read_to_string(artifact_dir.join("profile.yaml"))
            .await
            .ok()
            .and_then(|text| serde_yaml::from_str::<ProfileManifest>(&text).ok())
            .filter(|manifest| manifest.validate_for_provider(self.provider).is_ok());
        let Some(manifest) = manifest else {
            diagnostic.reason(Code::CleanupQuarantined, StageId::Completion, true);
            return Ok(ReconcileStep::Settled(
                self.quarantine(id, now, "admitted manifest invalid for destroy")
                    .await?,
            ));
        };

        // Existing identity, or Phase 1 retry once the failed worker is fenced.
        let mut identity = self.lifecycle.generation_state_identity(id).await?;
        if identity.is_none() && anchor_failed_create(self.lifecycle, id).await.is_anchored() {
            identity = self.lifecycle.generation_state_identity(id).await?;
        }
        if let Some((lineage, serial)) = identity {
            return Ok(ReconcileStep::ReadyForDestroy(Box::new(DestroyProof {
                original_provenance,
                lineage,
                serial,
                bindings_digest,
                artifact_dir,
                managed_shape: manifest.managed_resource_shape,
            })));
        }

        // Phase 2: optional read-only discover + targeted import.
        let Some(reconcile_cfg) = manifest.reconcile else {
            diagnostic.reason(Code::CleanupQuarantined, StageId::Completion, true);
            return Ok(ReconcileStep::Settled(
                self.quarantine(id, now, "state identity missing for destroy")
                    .await?,
            ));
        };
        let settle_ms = i64::try_from(reconcile_cfg.settle_delay_secs)
            .unwrap_or(i64::MAX)
            .saturating_mul(1000);
        if settle_ms > 0 && now.saturating_sub(entered_destroying_at) < settle_ms {
            return Ok(ReconcileStep::Settled(DestroyOutcome::Pending));
        }

        let request = TemplateReconcileRequest {
            pinned_artifact_digest: generation.template_artifact_digest.clone(),
            workspace_path: std::path::PathBuf::from(&generation.workspace_path),
            artifact_dir: artifact_dir.clone(),
            expected_bindings_digest: BindingsDigest(bindings_digest.clone()),
            generation_id: generation.id.clone(),
            managed_shape: manifest.managed_resource_shape.clone(),
            discover_module: reconcile_cfg.discover_module,
            environment: Vec::new(),
            timeout: self.operation_timeout,
            original_provenance: original_provenance.clone(),
        };
        let outcome = self.runtime.reconcile(request).await;
        let at = self.now_after(now);
        match outcome {
            Ok(ReconcileClassification::Empty {
                state_lineage,
                state_serial,
            }) => {
                record_iac(MetricResult::Ok);
                self.destroy_backoff.clear(id);
                let anchored = self
                    .lifecycle
                    .generation_anchor_reconciled_create(id, &state_lineage, state_serial, true)
                    .await?;
                if !anchored {
                    diagnostic.reason(Code::CleanupQuarantined, StageId::Completion, true);
                    return Ok(ReconcileStep::Settled(
                        self.quarantine(id, at, "reconciled empty state could not be anchored")
                            .await?,
                    ));
                }
                self.lifecycle
                    .generation_advance(id, G::Destroyed, at)
                    .await?;
                diagnostic.reason(Code::CleanupCompleted, StageId::Completion, false);
                diagnostic.outcome(Outcome::Satisfied);
                if let Some(d) = &mut diagnostic.0 {
                    for r in &mut d.observation.question.reasons {
                        r.parameters.completion_source = Some(CompletionSource::ProviderCleanup);
                    }
                }
                Ok(ReconcileStep::Settled(DestroyOutcome::Destroyed))
            }
            Ok(ReconcileClassification::Anchored {
                state_lineage,
                state_serial,
                adopted_count,
            }) => {
                record_iac(MetricResult::Ok);
                self.destroy_backoff.clear(id);
                tracing::info!(
                    generation = %id,
                    adopted_count,
                    "failed create reconciled via discover; proceeding to destroy"
                );
                let anchored = self
                    .lifecycle
                    .generation_anchor_reconciled_create(id, &state_lineage, state_serial, false)
                    .await?;
                if !anchored {
                    diagnostic.reason(Code::CleanupQuarantined, StageId::Completion, true);
                    return Ok(ReconcileStep::Settled(
                        self.quarantine(id, at, "reconciled state could not be anchored")
                            .await?,
                    ));
                }
                Ok(ReconcileStep::ReadyForDestroy(Box::new(DestroyProof {
                    original_provenance,
                    lineage: state_lineage,
                    serial: state_serial,
                    bindings_digest,
                    artifact_dir,
                    managed_shape: manifest.managed_resource_shape,
                })))
            }
            Ok(ReconcileClassification::Unimportable { address }) => {
                tracing::warn!(
                    generation = %id,
                    %address,
                    "untracked remote resource has no import_id; quarantining"
                );
                diagnostic.reason(Code::CleanupQuarantined, StageId::Completion, true);
                Ok(ReconcileStep::Settled(
                    self.quarantine(id, at, "untracked remote resource cannot be imported")
                        .await?,
                ))
            }
            Err(TemplateOutcomeError::PlanFailed { phase }) => {
                record_iac(MetricResult::Failed);
                tracing::warn!(
                    generation = %id,
                    %phase,
                    "failed create reconcile contract rejected; quarantining"
                );
                diagnostic.reason(Code::CleanupQuarantined, StageId::Completion, true);
                Ok(ReconcileStep::Settled(
                    self.quarantine(id, at, "failed create reconcile rejected")
                        .await?,
                ))
            }
            Err(
                TemplateOutcomeError::ExecutionFailed { phase }
                | TemplateOutcomeError::StateUnavailable { phase },
            ) => {
                record_iac(MetricResult::Failed);
                let retry_in_ms = self.destroy_backoff.failed(id, at);
                tracing::warn!(
                    generation = %id,
                    %phase,
                    retry_in_ms,
                    "failed create reconcile transiently failed; will retry"
                );
                diagnostic.reason(
                    Code::LifecycleApplyOutcomeUnknown,
                    StageId::ResourceCleanup,
                    false,
                );
                Ok(ReconcileStep::Settled(DestroyOutcome::Pending))
            }
        }
    }
}
