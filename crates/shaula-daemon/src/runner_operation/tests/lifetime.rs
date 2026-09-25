//! Runner Maximum Lifetime: the Runner Resource first, then the registration.

use std::time::Duration;

use shaula_core::lifecycle::GenerationState as G;

use super::fakes::Scripted;
use super::fixture::{Backend, Fixture, TestResult, BACKENDS, FLEET};
use super::seed::Seed;
use crate::runner_operation::DestroyOutcome;

const NOW: i64 = 100_000;
const LIMIT: Duration = Duration::from_millis(1);

#[tokio::test]
async fn expiry_destroys_resources_before_registration_even_while_busy() -> TestResult {
    for backend in BACKENDS {
        let fixture = Fixture::new(backend).await?;
        fixture.seed(Seed::started(G::Busy)).await?;
        fixture.remote(Scripted::Busy)?;
        let operation = fixture.operation();
        let due = operation.expired(FLEET, NOW, LIMIT).await?;
        assert_eq!(due.len(), 1, "{backend:?}");
        let outcome = operation.expire(&due[0], NOW).await?;
        assert_eq!(fixture.destroys(), 1, "{backend:?}");
        match backend {
            // GitHub still refuses deregistration while the job runs remotely.
            Backend::Github => {
                assert_eq!(outcome, DestroyOutcome::Pending);
                assert_eq!(fixture.state().await?, G::Destroying);
                fixture.remote(Scripted::Idle)?;
                let generation = fixture.generation().await?;
                let outcome = operation.expire(&generation, NOW).await?;
                assert_eq!(outcome, DestroyOutcome::Destroyed);
            }
            // Forgejo has no remote busy protection; the resource is already gone.
            Backend::Forgejo => assert_eq!(outcome, DestroyOutcome::Destroyed),
        }
        assert_eq!(fixture.state().await?, G::Destroyed, "{backend:?}");
        assert_eq!(fixture.destroys(), 1, "{backend:?}");
    }
    Ok(())
}

#[tokio::test]
async fn expiry_without_registration_identity_is_quarantined_after_resource_destroy() -> TestResult
{
    for backend in BACKENDS {
        let fixture = Fixture::new(backend).await?;
        let generation = fixture
            .seed(Seed {
                identity: false,
                ..Seed::started(G::Idle)
            })
            .await?;
        let outcome = fixture.operation().expire(&generation, NOW).await?;
        assert_eq!(outcome, DestroyOutcome::Quarantined, "{backend:?}");
        assert_eq!(fixture.destroys(), 1, "{backend:?}");
    }
    Ok(())
}

#[tokio::test]
async fn expiry_with_missing_destroy_proof_quarantines_before_any_effect() -> TestResult {
    for backend in BACKENDS {
        let fixture = Fixture::new(backend).await?;
        let generation = fixture.seed(Seed::started(G::Idle)).await?;
        fixture.corrupt_manifest()?;
        let outcome = fixture.operation().expire(&generation, NOW).await?;
        assert_eq!(outcome, DestroyOutcome::Quarantined, "{backend:?}");
        assert_eq!(
            (fixture.removals(), fixture.destroys()),
            (0, 0),
            "{backend:?}"
        );
    }
    Ok(())
}
