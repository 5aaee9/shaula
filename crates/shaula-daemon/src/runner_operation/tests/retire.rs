//! Retirement: busy-safe registration removal, then the Runner Resource.

use std::sync::atomic::Ordering;

use shaula_core::lifecycle::GenerationState as G;
use shaula_core::registry::LifecycleStore;

use super::fakes::Scripted;
use super::fixture::{Backend, Fixture, TestResult, BACKENDS, GENERATION};
use super::seed::Seed;
use crate::runner_operation::DestroyOutcome;

const NOW: i64 = 100_000;

/// State after a refused removal of an Idle Generation. GitHub's gate is the
/// removal call itself, so Retirement is already recorded; Forgejo proves
/// idleness read-only first, so an unproven runner keeps serving as Idle.
fn after_refusal(backend: Backend) -> G {
    match backend {
        Backend::Github => G::Retiring,
        Backend::Forgejo => G::Idle,
    }
}

#[tokio::test]
async fn idle_generation_is_destroyed_after_its_registration_is_removed() -> TestResult {
    for backend in BACKENDS {
        let fixture = Fixture::new(backend).await?;
        let generation = fixture.seed(Seed::started(G::Idle)).await?;
        fixture.remote(Scripted::Idle)?;
        let outcome = fixture.operation().retire(&generation, NOW).await?;
        assert_eq!(outcome, DestroyOutcome::Destroyed, "{backend:?}");
        assert_eq!(fixture.state().await?, G::Destroyed, "{backend:?}");
        assert_eq!(
            (fixture.removals(), fixture.destroys()),
            (1, 1),
            "{backend:?}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn busy_registration_keeps_retirement_pending_and_resources_intact() -> TestResult {
    for backend in BACKENDS {
        let fixture = Fixture::new(backend).await?;
        let generation = fixture.seed(Seed::started(G::Idle)).await?;
        fixture.remote(Scripted::Busy)?;
        let outcome = fixture.operation().retire(&generation, NOW).await?;
        assert_eq!(outcome, DestroyOutcome::Pending, "{backend:?}");
        assert_eq!(
            fixture.state().await?,
            after_refusal(backend),
            "{backend:?}"
        );
        assert_eq!(fixture.destroys(), 0, "{backend:?}");

        // The next tick re-gates the same Generation.
        fixture.remote(Scripted::Idle)?;
        let generation = fixture.generation().await?;
        let outcome = fixture.operation().retire(&generation, NOW).await?;
        assert_eq!(outcome, DestroyOutcome::Destroyed, "{backend:?}");
        assert_eq!(fixture.destroys(), 1, "{backend:?}");
    }
    Ok(())
}

#[tokio::test]
async fn unavailable_registration_observation_retries_instead_of_quarantining() -> TestResult {
    for backend in BACKENDS {
        let fixture = Fixture::new(backend).await?;
        let generation = fixture.seed(Seed::started(G::Idle)).await?;
        fixture.remote(Scripted::Unavailable)?;
        let outcome = fixture.operation().retire(&generation, NOW).await?;
        assert_eq!(outcome, DestroyOutcome::Pending, "{backend:?}");
        assert_eq!(
            fixture.state().await?,
            after_refusal(backend),
            "{backend:?}"
        );
        assert_eq!(fixture.destroys(), 0, "{backend:?}");
    }
    Ok(())
}

/// ARD-0040: no Create `ApplyStarting` means no Runner Resource exists.
#[tokio::test]
async fn never_started_generation_is_destroyed_without_a_terraform_destroy() -> TestResult {
    for backend in BACKENDS {
        let fixture = Fixture::new(backend).await?;
        let generation = fixture
            .seed(Seed::never_started(G::CleanupRequired))
            .await?;
        fixture.remote_absent()?;
        let outcome = fixture.operation().retire(&generation, NOW).await?;
        assert_eq!(outcome, DestroyOutcome::Destroyed, "{backend:?}");
        assert_eq!(fixture.state().await?, G::Destroyed, "{backend:?}");
        assert_eq!(fixture.destroys(), 0, "{backend:?}");
    }
    Ok(())
}

#[tokio::test]
async fn never_started_generation_removes_its_recorded_registration_first() -> TestResult {
    for backend in BACKENDS {
        let fixture = Fixture::new(backend).await?;
        let generation = fixture
            .seed(Seed {
                identity: true,
                ..Seed::never_started(G::CleanupRequired)
            })
            .await?;
        fixture.remote(Scripted::Idle)?;
        let outcome = fixture.operation().retire(&generation, NOW).await?;
        assert_eq!(outcome, DestroyOutcome::Destroyed, "{backend:?}");
        assert_eq!(
            (fixture.removals(), fixture.destroys()),
            (1, 0),
            "{backend:?}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn never_started_generation_with_a_name_collision_is_quarantined() -> TestResult {
    for backend in BACKENDS {
        let fixture = Fixture::new(backend).await?;
        let generation = fixture
            .seed(Seed::never_started(G::CleanupRequired))
            .await?;
        fixture.remote(Scripted::NameTaken)?;
        let outcome = fixture.operation().retire(&generation, NOW).await?;
        assert_eq!(outcome, DestroyOutcome::Quarantined, "{backend:?}");
        assert_eq!(fixture.destroys(), 0, "{backend:?}");
    }
    Ok(())
}

#[tokio::test]
async fn open_create_intent_blocks_the_never_started_shortcut() -> TestResult {
    for backend in BACKENDS {
        let fixture = Fixture::new(backend).await?;
        let generation = fixture
            .seed(Seed::never_started(G::CleanupRequired))
            .await?;
        fixture
            .operation_row("Create", "ApplyStarting", None)
            .await?;
        fixture.remote_absent()?;
        // A started Create always retains its registration identity.
        let outcome = fixture.operation().retire(&generation, NOW).await?;
        assert_eq!(outcome, DestroyOutcome::Quarantined, "{backend:?}");
        assert_eq!(fixture.destroys(), 0, "{backend:?}");
    }
    Ok(())
}

#[tokio::test]
async fn unresolved_forgejo_registration_intent_is_quarantined() -> TestResult {
    let fixture = Fixture::new(Backend::Forgejo).await?;
    let generation = fixture
        .seed(Seed::never_started(G::CleanupRequired))
        .await?;
    fixture
        .operation_row("ForgejoRegistration", "Starting", None)
        .await?;
    fixture.remote_absent()?;
    let outcome = fixture.operation().retire(&generation, NOW).await?;
    assert_eq!(outcome, DestroyOutcome::Quarantined);
    Ok(())
}

#[tokio::test]
async fn missing_destroy_proof_quarantines_instead_of_aborting_the_pass() -> TestResult {
    let cases = [
        (
            "state identity",
            Seed {
                state_identity: false,
                ..Seed::started(G::Idle)
            },
            false,
        ),
        (
            "admitted bindings",
            Seed {
                template_revision: 9,
                ..Seed::started(G::Idle)
            },
            false,
        ),
        ("admitted manifest", Seed::started(G::Idle), true),
    ];
    for backend in BACKENDS {
        for (missing, seed, corrupt_manifest) in cases {
            let fixture = Fixture::new(backend).await?;
            let generation = fixture.seed(seed).await?;
            if corrupt_manifest {
                fixture.corrupt_manifest()?;
            }
            fixture.remote(Scripted::Idle)?;
            let outcome = fixture.operation().retire(&generation, NOW).await?;
            assert_eq!(
                outcome,
                DestroyOutcome::Quarantined,
                "{backend:?}: {missing}"
            );
            assert_eq!(
                fixture.state().await?,
                G::Quarantined,
                "{backend:?}: {missing}"
            );
            assert_eq!(fixture.destroys(), 0, "{backend:?}: {missing}");
        }
    }
    Ok(())
}

#[tokio::test]
async fn failed_resource_destroy_resumes_without_repeating_registration_removal() -> TestResult {
    for backend in BACKENDS {
        let fixture = Fixture::new(backend).await?;
        let generation = fixture.seed(Seed::started(G::Idle)).await?;
        fixture.remote(Scripted::Idle)?;
        fixture.runtime.destroy_ok.store(false, Ordering::SeqCst);
        let outcome = fixture.operation().retire(&generation, NOW).await?;
        assert_eq!(outcome, DestroyOutcome::Pending, "{backend:?}");
        assert_eq!(fixture.state().await?, G::DestroyPending, "{backend:?}");

        fixture.runtime.destroy_ok.store(true, Ordering::SeqCst);
        let generation = fixture.generation().await?;
        let outcome = fixture.operation().retire(&generation, NOW).await?;
        assert_eq!(outcome, DestroyOutcome::Destroyed, "{backend:?}");
        assert_eq!(
            (fixture.removals(), fixture.destroys()),
            (1, 2),
            "{backend:?}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn committed_expiry_is_left_to_the_lifetime_path() -> TestResult {
    for backend in BACKENDS {
        let fixture = Fixture::new(backend).await?;
        let generation = fixture.seed(Seed::started(G::Idle)).await?;
        let lifecycle: &dyn LifecycleStore = fixture.store.as_ref();
        assert!(lifecycle.generation_request_expiry(GENERATION, NOW).await?);
        let outcome = fixture.operation().retire(&generation, NOW).await?;
        assert_eq!(outcome, DestroyOutcome::Pending, "{backend:?}");
        assert_eq!(fixture.state().await?, G::Idle, "{backend:?}");
        assert_eq!(fixture.destroys(), 0, "{backend:?}");
    }
    Ok(())
}
