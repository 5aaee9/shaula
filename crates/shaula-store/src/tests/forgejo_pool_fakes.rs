use async_trait::async_trait;
use shaula_core::{ports::forgejo::*, ports::*, secret::SecretString};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Mutex,
};

#[derive(Default)]
pub(super) struct Forgejo {
    pub runners: Mutex<Vec<ForgejoRunnerRef>>,
    pub waiting: AtomicU64,
    pub posts: AtomicU64,
    pub deletes: AtomicU64,
    pub fail_jobs: AtomicBool,
    pub fail_inventory: AtomicBool,
    pub uncertain: AtomicBool,
}
#[async_trait]
impl ForgejoPoolPort for Forgejo {
    async fn register_runner(
        &self,
        name: &str,
        _: Option<&str>,
    ) -> Result<EffectOutcome<ForgejoRegistration>, AccessFailure> {
        let id = self.posts.fetch_add(1, Ordering::SeqCst) + 1;
        let uuid = format!("uuid-{id}");
        self.runners
            .lock()
            .map_err(|_| failure())?
            .push(ForgejoRunnerRef {
                id,
                uuid: uuid.clone(),
                name: name.into(),
                status: "offline".into(),
                labels: vec![],
                ephemeral: true,
                version: None,
            });
        if self.uncertain.load(Ordering::SeqCst) {
            return Ok(EffectOutcome::Uncertain {
                summary: "lost response".into(),
            });
        }
        Ok(EffectOutcome::Definite(ForgejoRegistration {
            id,
            uuid,
            token: SecretString::new("one-shot-token"),
        }))
    }
    async fn list_runners(&self) -> Result<Vec<ForgejoRunnerRef>, AccessFailure> {
        if self.fail_inventory.load(Ordering::SeqCst) {
            return Err(failure());
        }
        Ok(self.runners.lock().map_err(|_| failure())?.clone())
    }
    async fn get_runner(&self, id: u64) -> Result<Option<ForgejoRunnerRef>, AccessFailure> {
        Ok(self
            .list_runners()
            .await?
            .into_iter()
            .find(|runner| runner.id == id))
    }
    async fn list_jobs(&self, labels: &[String]) -> Result<Vec<ForgejoJob>, AccessFailure> {
        assert_eq!(labels, ["linux"]);
        if self.fail_jobs.load(Ordering::SeqCst) {
            return Err(failure());
        }
        Ok((0..self.waiting.load(Ordering::SeqCst))
            .map(|id| ForgejoJob {
                id,
                handle: id.to_string(),
                attempt: 1,
                status: "waiting".into(),
                runs_on: labels.to_vec(),
                task_id: 0,
                run_id: 1,
                repo_id: 1,
                name: "test".into(),
            })
            .collect())
    }
    async fn delete_runner(&self, id: u64) -> Result<ForgejoRemovalOutcome, AccessFailure> {
        self.deletes.fetch_add(1, Ordering::SeqCst);
        self.runners
            .lock()
            .map_err(|_| failure())?
            .retain(|r| r.id != id);
        Ok(ForgejoRemovalOutcome::Removed)
    }
    async fn classify_uncertain_registration(
        &self,
        name: &str,
        _: &[String],
    ) -> Result<ForgejoRegistrationUncertainty, AccessFailure> {
        Ok(
            if self.list_runners().await?.iter().any(|r| r.name == name) {
                ForgejoRegistrationUncertainty::Quarantined
            } else {
                ForgejoRegistrationUncertainty::None
            },
        )
    }
    async fn auth_probe(&self) -> Result<ForgejoAuthProbe, AccessFailure> {
        Err(failure())
    }
}
fn failure() -> AccessFailure {
    AccessFailure::Unavailable {
        summary: "injected".into(),
    }
}

#[derive(Default)]
pub(super) struct Runtime {
    pub creates: AtomicU64,
    pub destroys: AtomicU64,
    pub idle_proof: AtomicBool,
    pub fail_prepare: AtomicBool,
}
#[async_trait]
impl TemplateRuntimePort for Runtime {
    async fn prepare_create(
        &self,
        _: &std::path::Path,
        _: &std::path::Path,
        _: &str,
        _: std::time::Duration,
    ) -> Result<(), TemplateOutcomeError> {
        if self.fail_prepare.load(Ordering::SeqCst) {
            return Err(runtime_error("prepare"));
        }
        Ok(())
    }
    async fn create(
        &self,
        request: TemplateCreateRequest,
    ) -> Result<TemplateCreateResult, TemplateOutcomeError> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        let material = request
            .forgejo_bootstrap
            .as_ref()
            .ok_or_else(|| runtime_error("missing token"))?;
        assert!(!request
            .input
            .to_tfvars()
            .map_err(|_| runtime_error("input"))?
            .contains(material.token()));
        assert!(request.input.generation.scale_set_id.is_none());
        let provenance = provenance(&request.input.generation.id);
        let sink = request
            .apply_intent_sink
            .as_ref()
            .ok_or_else(|| runtime_error("sink"))?;
        let claim = sink
            .persist_apply_starting(&provenance)
            .await
            .map_err(|_| runtime_error("fence"))?;
        drop(claim);
        let claim = sink
            .authorize_bootstrap(&provenance)
            .await
            .map_err(|_| runtime_error("bootstrap"))?;
        drop(claim);
        Ok(TemplateCreateResult {
            result_envelope: shaula_core::template::ShaulaResultEnvelope {
                contract_version: 1,
                generation_id: request.input.generation.id,
                bindings_digest: request.expected_bindings_digest,
                resources: vec![],
            },
            state_lineage: "lineage".into(),
            state_serial: 1,
            provenance,
        })
    }
    async fn destroy(
        &self,
        request: TemplateDestroyRequest,
    ) -> Result<DestroyClassification, TemplateOutcomeError> {
        self.destroys.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request.original_state.lineage, "lineage");
        Ok(DestroyClassification::Applied)
    }
    async fn forgejo_template_evidence(&self, _: &str) -> ForgejoTemplateEvidence {
        if self.idle_proof.load(Ordering::SeqCst) {
            ForgejoTemplateEvidence::ProcessIdle
        } else {
            ForgejoTemplateEvidence::Unknown
        }
    }
}
fn runtime_error(phase: &str) -> TemplateOutcomeError {
    TemplateOutcomeError::ExecutionFailed {
        phase: phase.into(),
    }
}
pub(super) fn provenance(id: &str) -> PlanProvenance {
    PlanProvenance {
        intent: shaula_core::plan::PlanIntent::Create,
        saved_plan_digest: "sha256:plan".into(),
        engine_kind: "terraform".into(),
        engine_version: "1.9.8".into(),
        engine_binary_digest: "sha256:engine".into(),
        artifact_digest: format!("sha256:{}", "a".repeat(64)),
        template_material_digest: "sha256:material".into(),
        protected_input_digest: "sha256:input".into(),
        state_lineage: StateLineage::Empty,
        generation_id: id.into(),
        attempt_id: format!("create-{id}"),
    }
}
