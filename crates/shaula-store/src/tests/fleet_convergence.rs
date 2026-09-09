//! The public Fleet status port distinguishes listener readiness from capacity.
use std::sync::Arc;

use sea_orm::{ConnectionTrait, DbBackend, Statement};
use shaula_core::lifecycle::GenerationState;
use shaula_core::ports::Clock;
use shaula_core::registry::{Actor, FleetRegistryPort, GenerationRecord, Scope};
use shaula_daemon::service::ControlPlane;

struct FixedClock;
impl Clock for FixedClock {
    fn now_unix_ms(&self) -> i64 {
        100
    }
}

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[tokio::test]
async fn ready_fleet_converges_only_when_effective_and_occupied_capacity_match_target() -> TestResult
{
    for (demand, states, expected) in [
        (0, vec![], true),
        (2, vec![], false),
        (1, vec![GenerationState::WaitingOnline], true),
        (
            1,
            vec![GenerationState::Idle, GenerationState::Retiring],
            false,
        ),
        (0, vec![GenerationState::Retiring], false),
    ] {
        let (store, _) = super::runtime_sessions::established().await?;
        let spec = serde_json::json!({
            "github": {"target":{"kind":"organization","owner":"example-org"},
                "auth_profile_ref":"shared-github", "scale_set_name":"test",
                "runner_group":"Default", "labels":[]},
            "capacity":{"min_runners":0,"max_runners":5},
            "template_profile_ref":"test", "template_inputs":{}
        });
        store
            .connection()
            .execute(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "UPDATE fleet_revisions SET spec_json=? WHERE fleet_key='fleet'",
                [spec.to_string().into()],
            ))
            .await?;
        store.connection().execute_unprepared("UPDATE fleets SET phase='Ready',observed_revision=desired_revision WHERE key='fleet'").await?;
        store.demand_snapshot("fleet", demand, 20).await?;
        for (index, state) in states.into_iter().enumerate() {
            store
                .generation_insert(GenerationRecord {
                    id: format!("generation-{index}"),
                    fleet_key: "fleet".into(),
                    runner_name: format!("runner-{index}"),
                    generation_name: format!("generation-{index}"),
                    fleet_revision: 1,
                    template_profile_key: "test".into(),
                    template_revision: 1,
                    template_artifact_digest: "sha256:test".into(),
                    attestation_id: "test".into(),
                    inputs_digest: "sha256:inputs".into(),
                    state,
                    github_runner_id: None,
                    workspace_path: format!("test-{index}"),
                    created_at: 20,
                    updated_at: 20,
                })
                .await?;
        }
        let backend = Arc::new(crate::registry_impl::SqliteControlPlane::new(
            store,
            "unused".into(),
        ));
        let service = ControlPlane::new(
            backend.clone(),
            Arc::new(FixedClock),
            vec![],
            100,
            "unused".into(),
        );
        let actor = Actor {
            name: "operator".into(),
            scopes: vec![Scope::FleetRead],
        };
        let status = service
            .fleet_status_get(&actor, "fleet")
            .await?
            .map_err(|e| format!("{e:?}"))?;
        assert_eq!(status.phase, "Ready");
        assert_eq!(status.capacity.target, demand);
        assert_eq!(
            status
                .conditions
                .iter()
                .find(|c| c.condition_type == "Converged")
                .ok_or("condition")?
                .status,
            expected
        );
        // Matching capacity does not replace the revision fence or Ready phase.
        for mutation in [
            "UPDATE fleets SET observed_revision=0 WHERE key='fleet'",
            "UPDATE fleets SET observed_revision=desired_revision,phase='Degraded' WHERE key='fleet'",
        ] {
            backend.store().connection().execute_unprepared(mutation).await?;
            let status = service.fleet_status_get(&actor,"fleet").await?.map_err(|e|format!("{e:?}"))?;
            assert!(!status.conditions.iter().find(|c|c.condition_type=="Converged").ok_or("condition")?.status);
        }
    }
    Ok(())
}
