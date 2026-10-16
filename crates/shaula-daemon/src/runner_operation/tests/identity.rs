//! A refresh that adopted another Generation's resource (a reused Proxmox
//! VMID, molecule 2026-10-16) is proof the bound resource is gone: retrying
//! only waits for the next reuse. Both destroy paths must quarantine at once.

use std::sync::atomic::Ordering;
use std::time::Duration;

use shaula_core::lifecycle::GenerationState as G;

use super::fakes::Scripted;
use super::fixture::{Fixture, TestResult, BACKENDS, FLEET};
use super::seed::Seed;
use crate::runner_operation::DestroyOutcome;

const NOW: i64 = 100_000;

#[tokio::test]
async fn retirement_quarantines_a_destroy_refused_by_identity() -> TestResult {
    for backend in BACKENDS {
        let fixture = Fixture::new(backend).await?;
        let generation = fixture.seed(Seed::started(G::Idle)).await?;
        fixture.remote(Scripted::Idle)?;
        fixture
            .runtime
            .destroy_foreign
            .store(true, Ordering::SeqCst);
        let outcome = fixture.operation().retire(&generation, NOW).await?;
        assert_eq!(outcome, DestroyOutcome::Quarantined, "{backend:?}");
        assert_eq!(fixture.state().await?, G::Quarantined, "{backend:?}");
        assert_eq!(fixture.destroys(), 1, "{backend:?}");
    }
    Ok(())
}

#[tokio::test]
async fn expiry_quarantines_a_destroy_refused_by_identity() -> TestResult {
    for backend in BACKENDS {
        let fixture = Fixture::new(backend).await?;
        fixture.seed(Seed::started(G::Busy)).await?;
        fixture
            .runtime
            .destroy_foreign
            .store(true, Ordering::SeqCst);
        let operation = fixture.operation();
        let due = operation
            .expired(FLEET, NOW, Duration::from_millis(1))
            .await?;
        let outcome = operation.expire(&due[0], NOW).await?;
        assert_eq!(outcome, DestroyOutcome::Quarantined, "{backend:?}");
        assert_eq!(fixture.state().await?, G::Quarantined, "{backend:?}");
        assert_eq!(fixture.removals(), 0, "{backend:?}");
    }
    Ok(())
}
