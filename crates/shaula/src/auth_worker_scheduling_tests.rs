//! G1/G2 scheduling composition tests: the REAL `SupervisorWiring` tick
//! loop drives the REAL v2 worker against the scripted GitHub mock — the
//! production scheduling path itself, no test-only dispatch. Time is
//! controlled through the tick parameter while the worker's fixed clock
//! anchors deferral deadlines, so Retry-After windows are crossed
//! deterministically.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::auth_worker_v2::tests::{control_plane, policy, seed_candidate, KEY};
use crate::wiring::wiring_tests::{tick_and_drain, wiring_with};
use shaula_core::registry::ControlPlaneStore;
use std::sync::atomic::Ordering;

/// The worker clock's fixed instant (auth_worker_mock::Now).
const T0: i64 = 1_800_000_000_000;

const NOLIMIT_POLICY: &str =
    r#"{"selectors":[{"kind":"account_repositories","account_kind":"user","owner":"nolimit"}]}"#;

/// G1: the scheduling loop passes the REAL Profile key to the worker —
/// `auth/{key}` is only the internal task name. A candidate published
/// under `shared-github` promotes through `tick_all`; with the old bug
/// (worker keyed `auth/shared-github`) the worker would find no profile
/// and the candidate would stay Validating forever.
#[tokio::test]
async fn scheduling_passes_real_profile_key_and_promotes_candidate() {
    let mock = crate::auth_worker_mock::mock_server(false).await;
    let control_plane = control_plane().await;
    seed_candidate(&control_plane, 1, &policy(false)).await;
    let mut wiring = wiring_with(
        &control_plane,
        crate::auth_worker_mock::endpoints(&mock.base),
    )
    .await;
    tick_and_drain(&mut wiring, T0).await;
    let head = ControlPlaneStore::auth_profile_get(control_plane.as_ref(), KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        head.active_revision,
        Some(1),
        "the real profile key must reach the worker through the scheduling loop"
    );
    assert_eq!(head.status, "Active");
    assert!(
        !mock.user_agents.lock().unwrap().is_empty(),
        "the scheduled worker made real GitHub requests"
    );
}

/// G2: a rate-limited validation carries GitHub's Retry-After as an
/// ABSOLUTE deadline — no worker attempt inside the window, a real
/// re-attempt after it, and the Candidate stays Pending throughout.
#[tokio::test]
async fn scheduling_honors_retry_after_deadline() {
    let mock = crate::auth_worker_mock::mock_server(false).await;
    let control_plane = control_plane().await;
    seed_candidate(&control_plane, 1, NOLIMIT_POLICY).await;
    let mut wiring = wiring_with(
        &control_plane,
        crate::auth_worker_mock::endpoints(&mock.base),
    )
    .await;
    tick_and_drain(&mut wiring, T0).await;
    assert_eq!(
        mock.discovery_reads.load(Ordering::SeqCst),
        1,
        "the first pass validates immediately"
    );
    tick_and_drain(&mut wiring, T0 + 30_000).await;
    assert_eq!(
        mock.discovery_reads.load(Ordering::SeqCst),
        1,
        "Retry-After: 120 gates re-attempts for the full window"
    );
    tick_and_drain(&mut wiring, T0 + 121_000).await;
    assert_eq!(
        mock.discovery_reads.load(Ordering::SeqCst),
        2,
        "the worker re-attempts after the Retry-After deadline"
    );
    let head = ControlPlaneStore::auth_profile_get(control_plane.as_ref(), KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        head.status, "Validating",
        "rate-limited validation stays Pending"
    );
    assert_eq!(head.active_revision, None);
}

/// G2: a transient failure with NO Retry-After header defers for the
/// bounded DEFAULT backoff — the candidate recovers instead of being
/// parked for the rest of the process (the old i64::MAX/2 deadline).
#[tokio::test]
async fn scheduling_transient_without_retry_after_recovers_bounded() {
    let mock = crate::auth_worker_mock::mock_server(false).await;
    let control_plane = control_plane().await;
    seed_candidate(
        &control_plane,
        1,
        r#"{"selectors":[{"kind":"account_repositories","account_kind":"user","owner":"flaky"}]}"#,
    )
    .await;
    let mut wiring = wiring_with(
        &control_plane,
        crate::auth_worker_mock::endpoints(&mock.base),
    )
    .await;
    tick_and_drain(&mut wiring, T0).await;
    assert_eq!(mock.discovery_reads.load(Ordering::SeqCst), 1);
    tick_and_drain(&mut wiring, T0 + 5_000).await;
    assert_eq!(
        mock.discovery_reads.load(Ordering::SeqCst),
        1,
        "a no-header transient waits out the bounded backoff"
    );
    tick_and_drain(&mut wiring, T0 + 30_001).await;
    assert_eq!(
        mock.discovery_reads.load(Ordering::SeqCst),
        2,
        "recovery happens after the bounded default backoff"
    );
    assert_eq!(
        control_plane
            .auth_profile_get(KEY)
            .await
            .unwrap()
            .unwrap()
            .active_revision,
        Some(1)
    );
}

/// G2: the deferral gate is keyed by the CANDIDATE REF (profile +
/// desired revision) — a newly published revision of the same profile
/// validates immediately instead of inheriting the superseded
/// revision's Retry-After deadline.
#[tokio::test]
async fn scheduling_new_revision_not_stranded_by_old_deferral() {
    let mock = crate::auth_worker_mock::mock_server(false).await;
    let control_plane = control_plane().await;
    seed_candidate(&control_plane, 1, NOLIMIT_POLICY).await;
    let mut wiring = wiring_with(
        &control_plane,
        crate::auth_worker_mock::endpoints(&mock.base),
    )
    .await;
    tick_and_drain(&mut wiring, T0).await;
    assert_eq!(
        mock.discovery_reads.load(Ordering::SeqCst),
        1,
        "revision 1 deferred by Retry-After: 120"
    );
    seed_candidate(&control_plane, 2, &policy(false)).await;
    tick_and_drain(&mut wiring, T0 + 1_000).await;
    let head = ControlPlaneStore::auth_profile_get(control_plane.as_ref(), KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(head.desired_revision, 2);
    assert_eq!(
        head.active_revision,
        Some(2),
        "a new revision validates immediately — never stranded by the old revision's deferral"
    );
}

#[tokio::test]
async fn scheduling_keeps_other_profiles_retry_windows() {
    let mock = crate::auth_worker_mock::mock_server(false).await;
    let control_plane = control_plane().await;
    for key in [KEY, "second-app"] {
        crate::auth_worker_v2::tests::seed_candidate_as(&control_plane, key, 1, NOLIMIT_POLICY)
            .await;
    }
    let mut wiring = wiring_with(
        &control_plane,
        crate::auth_worker_mock::endpoints(&mock.base),
    )
    .await;
    tick_and_drain(&mut wiring, T0).await;
    assert_eq!(mock.discovery_reads.load(Ordering::SeqCst), 2);
    tick_and_drain(&mut wiring, T0 + 1_000).await;
    assert_eq!(mock.discovery_reads.load(Ordering::SeqCst), 2);
    tick_and_drain(&mut wiring, T0 + 121_000).await;
    assert_eq!(mock.discovery_reads.load(Ordering::SeqCst), 4);
    tick_and_drain(&mut wiring, T0 + 122_000).await;
    assert_eq!(mock.discovery_reads.load(Ordering::SeqCst), 4);
}
