//! Ledger seeding: a Generation reaches its starting state only through the
//! store's own legal transitions and durable records.

use shaula_core::lifecycle::GenerationState as G;
use shaula_core::ports::{PlanProvenance, StateLineage};
use shaula_core::registry::{GenerationRecord, LifecycleStore, OperationInsert};

use super::fixture::{
    digest, Backend, Fixture, TestResult, FLEET, GENERATION, RUNNER_ID, RUNNER_NAME,
};

/// How the ledger records a Generation before the step under test.
#[derive(Debug, Clone, Copy)]
pub(super) struct Seed {
    pub state: G,
    pub create_started: bool,
    pub identity: bool,
    pub state_identity: bool,
    pub template_revision: i64,
}

impl Seed {
    /// A completed Create: apply admitted, registration and state identity kept.
    pub fn started(state: G) -> Self {
        Self {
            state,
            create_started: true,
            identity: true,
            state_identity: true,
            template_revision: 1,
        }
    }

    /// No Create apply, no registration identity, no state identity.
    pub fn never_started(state: G) -> Self {
        Self {
            state,
            create_started: false,
            identity: false,
            state_identity: false,
            template_revision: 1,
        }
    }
}

impl Fixture {
    pub async fn seed(&self, seed: Seed) -> TestResult<GenerationRecord> {
        let lifecycle: &dyn LifecycleStore = self.store.as_ref();
        lifecycle
            .generation_insert(GenerationRecord {
                id: GENERATION.into(),
                fleet_key: FLEET.into(),
                runner_name: RUNNER_NAME.into(),
                generation_name: GENERATION.into(),
                fleet_revision: 1,
                template_profile_key: "profile".into(),
                pool_member_key: None,
                template_revision: seed.template_revision,
                template_artifact_digest: digest(),
                attestation_id: "attestation".into(),
                inputs_digest: "inputs".into(),
                state: G::CreatePending,
                github_runner_id: None,
                workspace_path: self.artifact_root.join("work").to_string_lossy().into(),
                created_at: 1,
                updated_at: 1,
            })
            .await?;
        if seed.create_started {
            let provenance = PlanProvenance {
                intent: shaula_core::plan::PlanIntent::Create,
                saved_plan_digest: "sha256:plan".into(),
                engine_kind: "terraform".into(),
                engine_version: "1.9.8".into(),
                engine_binary_digest: "sha256:engine".into(),
                artifact_digest: digest(),
                template_material_digest: "sha256:material".into(),
                protected_input_digest: "sha256:input".into(),
                state_lineage: StateLineage::Empty,
                generation_id: GENERATION.into(),
                attempt_id: "create-attempt".into(),
            };
            self.operation_row(
                "Create",
                "Succeeded",
                Some(serde_json::to_string(&provenance)?),
            )
            .await?;
        }
        if seed.identity {
            self.record_identity().await?;
        }
        if seed.state_identity {
            let result =
                serde_json::json!({"result": {}, "state_lineage": "lineage", "state_serial": 3});
            lifecycle
                .generation_set_result(GENERATION, &result.to_string(), "sha256:result", 1)
                .await?;
        }
        for state in path_to(seed.state) {
            lifecycle.generation_advance(GENERATION, *state, 1).await?;
        }
        self.generation().await
    }

    async fn record_identity(&self) -> TestResult {
        let lifecycle: &dyn LifecycleStore = self.store.as_ref();
        match self.backend {
            Backend::Github => {
                lifecycle
                    .generation_set_github_runner(GENERATION, RUNNER_ID, 1)
                    .await?;
                // The admission-time Auth Revision Ref frozen with the JIT intent.
                let context = serde_json::json!({
                    "context": {"auth_profile_key": "github", "auth_revision": 1}
                });
                self.operation_row("JitStarting", "Succeeded", Some(context.to_string()))
                    .await?;
            }
            Backend::Forgejo => {
                lifecycle
                    .generation_set_forgejo_runner(GENERATION, RUNNER_ID, "uuid-55", 1)
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn operation_row(
        &self,
        kind: &str,
        state: &str,
        provenance_json: Option<String>,
    ) -> TestResult {
        let lifecycle: &dyn LifecycleStore = self.store.as_ref();
        lifecycle
            .operation_insert(OperationInsert {
                id: format!("{kind}-{GENERATION}"),
                generation_id: GENERATION.into(),
                kind: kind.into(),
                state: state.into(),
                provenance_json,
                saved_plan_path: None,
                saved_plan_digest: None,
                now: 1,
            })
            .await?;
        Ok(())
    }
}

/// Legal transitions from `CreatePending` to `state`.
fn path_to(state: G) -> &'static [G] {
    match state {
        G::Creating => &[G::Creating],
        G::WaitingOnline => &[G::Creating, G::WaitingOnline],
        G::Idle => &[G::Creating, G::WaitingOnline, G::Idle],
        G::Busy => &[G::Creating, G::WaitingOnline, G::Idle, G::Busy],
        G::CleanupRequired => &[G::Creating, G::CleanupRequired],
        G::Retiring => &[G::Creating, G::WaitingOnline, G::Idle, G::Retiring],
        G::DestroyPending => &[
            G::Creating,
            G::WaitingOnline,
            G::Idle,
            G::Retiring,
            G::DestroyPending,
        ],
        _ => &[],
    }
}
