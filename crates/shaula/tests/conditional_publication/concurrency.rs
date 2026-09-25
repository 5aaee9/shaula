use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Condvar, Mutex,
};
use std::time::Duration;

use shaula_core::ports::Clock;
use shaula_core::registry::{MutationAccepted, MutationError};
use shaula_daemon::service::ControlPlane;

use super::{registry::*, support::*};

// Both calls have completed replay, authority checks and admission before either
// may commit. Bounded waits make a sequencing regression fail, not hang the suite.
#[derive(Default)]
struct CommitBarrier {
    arrivals: Mutex<usize>,
    ready: Condvar,
    timed_out: AtomicBool,
}

impl Clock for CommitBarrier {
    #[expect(
        clippy::expect_used,
        reason = "a poisoned barrier is a failed test fixture"
    )]
    fn now_unix_ms(&self) -> i64 {
        let mut arrivals = self.arrivals.lock().expect("commit barrier poisoned");
        *arrivals += 1;
        self.ready.notify_all();
        let (_guard, wait) = self
            .ready
            .wait_timeout_while(arrivals, Duration::from_secs(5), |n| *n < 2)
            .expect("commit barrier poisoned");
        if wait.timed_out() {
            self.timed_out.store(true, Ordering::SeqCst);
        }
        1_800_000_003_000
    }
}

type Outcome = Result<MutationAccepted, MutationError>;

async fn race(
    fixture: &Fixture,
    kind: Kind,
    base: &MutationAccepted,
    variants: [u8; 2],
    keys: [&str; 2],
) -> TestResult<[Outcome; 2]> {
    let barrier = Arc::new(CommitBarrier::default());
    let plane = Arc::new(ControlPlane::new(
        fixture.store.clone(),
        barrier.clone(),
        b"test-bindings-key".to_vec(),
        100,
        fixture.engine.clone(),
    ));
    let spawn = |index: usize| -> TestResult<_> {
        let conditions = Conditions::replace(base, keys[index])?;
        let plane = plane.clone();
        let digest = fixture.digest.clone();
        Ok(tokio::spawn(async move {
            kind.publish(&plane, &actor(), &digest, variants[index], conditions)
                .await
        }))
    };
    let left = spawn(0)?;
    let right = spawn(1)?;
    let (left, right) = tokio::join!(left, right);
    assert!(
        !barrier.timed_out.load(Ordering::SeqCst),
        "{kind:?}: did not admit both requests"
    );
    assert_eq!(
        *barrier.arrivals.lock().map_err(|_| "barrier poisoned")?,
        2,
        "must not retry writes"
    );
    Ok([left??, right??])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn simultaneous_identical_publications_replay_one_winner_for_all_adapters() -> TestResult {
    let fixture = Fixture::new().await?;
    for kind in KINDS {
        let base = success(
            kind.publish(
                &fixture.service,
                &actor(),
                &fixture.digest,
                0,
                Conditions::create(),
            )
            .await?,
        )?;
        let outcomes = race(&fixture, kind, &base, [1, 1], ["same-key"; 2]).await?;
        assert!(outcomes[0].is_ok(), "{kind:?}: {outcomes:?}");
        assert_eq!(outcomes[0], outcomes[1], "{kind:?}");
        assert_eq!(kind.revision(&fixture).await?, 2);
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn simultaneous_different_bodies_or_protected_material_conflict_for_all_adapters(
) -> TestResult {
    let fixture = Fixture::new().await?;
    for kind in KINDS {
        let base = success(
            kind.publish(
                &fixture.service,
                &actor(),
                &fixture.digest,
                0,
                Conditions::create(),
            )
            .await?,
        )?;
        let outcomes = race(&fixture, kind, &base, [1, 2], ["same-key"; 2]).await?;
        assert_eq!(
            outcomes.iter().filter(|o| o.is_ok()).count(),
            1,
            "{kind:?}: {outcomes:?}"
        );
        assert!(
            outcomes.contains(&Err(MutationError::IdempotencyConflict)),
            "{kind:?}: {outcomes:?}"
        );
        assert_eq!(kind.revision(&fixture).await?, 2);
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn distinct_idempotency_keys_cannot_both_replace_the_same_head() -> TestResult {
    let fixture = Fixture::new().await?;
    for kind in KINDS {
        let base = success(
            kind.publish(
                &fixture.service,
                &actor(),
                &fixture.digest,
                0,
                Conditions::create(),
            )
            .await?,
        )?;
        let outcomes = race(&fixture, kind, &base, [1, 2], ["left", "right"]).await?;
        assert_eq!(
            outcomes.iter().filter(|o| o.is_ok()).count(),
            1,
            "{kind:?}: {outcomes:?}"
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|o| matches!(o, Err(MutationError::PreconditionFailed { .. })))
                .count(),
            1,
            "{kind:?}: {outcomes:?}"
        );
        assert_eq!(kind.revision(&fixture).await?, 2);
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn simultaneous_noops_replay_without_duplicate_audits_or_revisions() -> TestResult {
    let fixture = Fixture::new().await?;
    for kind in [Kind::Fleet, Kind::Pool, Kind::Template] {
        let base = success(
            kind.publish(
                &fixture.service,
                &actor(),
                &fixture.digest,
                0,
                Conditions::create(),
            )
            .await?,
        )?;
        let audits = fixture.count("audit_records").await?;
        let outcomes = race(&fixture, kind, &base, [0, 0], ["same-noop"; 2]).await?;
        assert!(
            outcomes[0].as_ref().is_ok_and(|a| a.no_op),
            "{kind:?}: {outcomes:?}"
        );
        assert_eq!(outcomes[0], outcomes[1], "{kind:?}");
        assert_eq!(kind.revision(&fixture).await?, 1);
        assert_eq!(fixture.count("audit_records").await?, audits + 1);
    }
    Ok(())
}
