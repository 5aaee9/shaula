//! Failed Runner Resource destroys back off exponentially (molecule: a refused
//! destroy plan re-planned every ~15s for 24 minutes). Failure modes covered:
//! retrying every tick; never retrying again; delaying a Generation that never
//! failed; an unbounded or overflowing delay; the lifetime path bypassing it.

use std::sync::atomic::Ordering;
use std::time::Duration;

use shaula_core::lifecycle::GenerationState as G;

use super::fakes::Scripted;
use super::fixture::{Fixture, TestResult, BACKENDS, FLEET};
use super::seed::Seed;
use crate::runner_operation::DestroyOutcome;

const NOW: i64 = 100_000;

#[tokio::test]
async fn failed_destroys_back_off_exponentially_up_to_a_cap() -> TestResult {
    for backend in BACKENDS {
        let fixture = Fixture::new(backend).await?;
        let generation = fixture.seed(Seed::started(G::Idle)).await?;
        fixture.remote(Scripted::Idle)?;
        fixture.runtime.destroy_ok.store(false, Ordering::SeqCst);
        // The first destroy is never delayed.
        fixture.operation().retire(&generation, NOW).await?;
        assert_eq!(fixture.destroys(), 1, "{backend:?}");

        let mut last = NOW;
        // 40 failures: the doubling saturates far past the cap without overflow.
        let delays = [15_000, 30_000, 60_000, 120_000, 240_000]
            .into_iter()
            .chain(std::iter::repeat(300_000))
            .take(40);
        for (attempt, delay) in delays.enumerate() {
            let generation = fixture.generation().await?;
            assert_eq!(generation.state, G::DestroyPending, "{backend:?}");
            let early = fixture
                .operation()
                .retire(&generation, last + delay - 1)
                .await?;
            assert_eq!(early, DestroyOutcome::Pending, "{backend:?}");
            assert_eq!(fixture.destroys(), attempt + 1, "{backend:?} early");
            last += delay;
            fixture.operation().retire(&generation, last).await?;
            assert_eq!(fixture.destroys(), attempt + 2, "{backend:?} due");
        }

        fixture.runtime.destroy_ok.store(true, Ordering::SeqCst);
        let generation = fixture.generation().await?;
        let outcome = fixture
            .operation()
            .retire(&generation, last + 300_000)
            .await?;
        assert_eq!(outcome, DestroyOutcome::Destroyed, "{backend:?}");
    }
    Ok(())
}

#[tokio::test]
async fn expiry_destroy_retries_honor_the_same_backoff() -> TestResult {
    for backend in BACKENDS {
        let fixture = Fixture::new(backend).await?;
        fixture.seed(Seed::started(G::Idle)).await?;
        fixture.remote(Scripted::Idle)?;
        fixture.runtime.destroy_ok.store(false, Ordering::SeqCst);
        let operation = fixture.operation();
        let limit = Duration::from_millis(1);
        let due = operation.expired(FLEET, NOW, limit).await?;
        assert_eq!(
            operation.expire(&due[0], NOW).await?,
            DestroyOutcome::Pending
        );
        assert_eq!(fixture.state().await?, G::DestroyPending, "{backend:?}");

        fixture.runtime.destroy_ok.store(true, Ordering::SeqCst);
        let due = operation.expired(FLEET, NOW + 1, limit).await?;
        assert_eq!(
            operation.expire(&due[0], NOW + 1).await?,
            DestroyOutcome::Pending
        );
        assert_eq!(fixture.destroys(), 1, "{backend:?}");

        let due = operation.expired(FLEET, NOW + 15_000, limit).await?;
        let outcome = operation.expire(&due[0], NOW + 15_000).await?;
        assert_eq!(outcome, DestroyOutcome::Destroyed, "{backend:?}");
        assert_eq!(fixture.destroys(), 2, "{backend:?}");
    }
    Ok(())
}
