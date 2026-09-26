//! Compare real lifecycle/store effects with optional diagnostics enabled/failed.
use super::{
    fakes::Scripted,
    fixture::{Fixture, TestResult, BACKENDS},
    seed::Seed,
};
use shaula_core::{
    diagnostics::*,
    lifecycle::GenerationState as G,
    registry::{Actor, Scope},
};

#[tokio::test]
async fn diagnostics_failure_preserves_lifecycle_effect_trace() -> TestResult {
    for backend in BACKENDS {
        for remote in [Scripted::Idle, Scripted::Busy, Scripted::Unavailable] {
            let mut baseline = None;
            for mode in 0..3 {
                let fixture = Fixture::new(backend).await?;
                let generation = fixture.seed(Seed::started(G::Idle)).await?;
                fixture.remote(remote)?;
                if mode == 2 {
                    fixture
                        .store
                        .store()
                        .execute_for_tests("DROP TABLE diagnostic_snapshots")
                        .await?;
                }
                let mut operation = fixture.operation();
                operation.diagnostic_guard = (mode != 0).then_some(&fixture.guard);
                let outcome = operation.retire(&generation, 100_000).await?;
                let trace = (
                    outcome,
                    fixture.state().await?,
                    fixture.removals(),
                    fixture.destroys(),
                );
                if let Some(expected) = baseline {
                    assert_eq!(trace, expected, "{backend:?} {remote:?}");
                } else {
                    baseline = Some(trace);
                }
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn diagnostics_no_create_completion_is_not_provider_destroy() -> TestResult {
    for backend in BACKENDS {
        let fixture = Fixture::new(backend).await?;
        let generation = fixture
            .seed(Seed::never_started(G::CleanupRequired))
            .await?;
        fixture.remote_absent()?;
        let mut operation = fixture.operation();
        operation.diagnostic_guard = Some(&fixture.guard);
        operation.retire(&generation, 100_000).await?;
        let actor = Actor {
            name: "reader".into(),
            authentication: Default::default(),
            scopes: vec![Scope::FleetRead],
        };
        let mut observed = false;
        for _ in 0..100 {
            let report = fixture
                .store
                .store()
                .diagnostics(SubjectKind::Generation, &generation.id, &actor, 100_001)
                .await?
                .ok_or("report")?;
            if report
                .questions
                .iter()
                .flat_map(|q| &q.reasons)
                .any(|r| r.parameters.completion_source == Some(CompletionSource::NeverStarted))
            {
                observed = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(observed, "{backend:?}");
        assert_eq!(fixture.destroys(), 0);
    }
    Ok(())
}
