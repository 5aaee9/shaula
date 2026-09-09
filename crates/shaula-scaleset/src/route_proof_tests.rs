//! Regression tests through the real GitHub and Actions HTTP adapter.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "route_proof_fixture.rs"]
mod fixture;
#[path = "route_proof_identity_tests.rs"]
mod identities;
#[path = "route_proof_race_tests.rs"]
mod races;
#[path = "route_proof_scope_tests.rs"]
mod scopes;

use fixture::Fixture;
use shaula_core::github::ScaleSetIdentity;
use shaula_core::ports::{AccessFailure, GitHubAccessPort};
use std::sync::atomic::Ordering;

#[tokio::test]
async fn proof_expires_and_refreshes_on_the_clock() {
    let f = Fixture::start().await;
    let client = f.client(false, true);
    let first = client.ensure_route_proof().await.unwrap();
    assert_eq!(first.valid_until_unix_ms, 1_060_000);
    f.advance(59_999);
    assert_eq!(client.ensure_route_proof().await.unwrap(), first);
    assert_eq!(f.reads(), 1);
    f.advance(1);
    assert!(
        client
            .ensure_route_proof()
            .await
            .unwrap()
            .checked_at_unix_ms
            > first.checked_at_unix_ms
    );
    assert_eq!(f.reads(), 2);
}

#[tokio::test]
async fn access_failure_invalidates_a_cached_positive_proof() {
    let f = Fixture::start().await;
    let client = f.client(false, true);
    client.ensure_route_proof().await.unwrap();
    f.script.groups_status.store(401, Ordering::SeqCst);
    assert!(client.probe_actions_access().await.is_err());
    assert!(matches!(
        client.ensure_route_proof().await,
        Err(AccessFailure::Unauthenticated)
    ));
    let reads = f.reads();
    let original_window = client
        .route_proof_failure_window(&AccessFailure::Unauthenticated)
        .await;
    assert_eq!(original_window, Some((1_000_000, 1_015_000)));
    f.advance(5_000);
    assert!(matches!(
        client.ensure_route_proof().await,
        Err(AccessFailure::Unauthenticated)
    ));
    assert_eq!(
        f.reads(),
        reads,
        "negative result retains its class and suppresses retry storms"
    );
    assert_eq!(
        client
            .route_proof_failure_window(&AccessFailure::Unauthenticated)
            .await,
        original_window
    );
    f.script.groups_status.store(200, Ordering::SeqCst);
    f.advance(10_000);
    client.ensure_route_proof().await.unwrap();
    assert_eq!(f.reads(), reads + 1);
}

#[tokio::test]
async fn acquisition_failures_force_reproof_before_the_next_effect() {
    for status in [401, 403, 404, 429] {
        let f = Fixture::start().await;
        let client = f.client(false, true);
        client.ensure_route_proof().await.unwrap();
        f.script.effect_status.store(status, Ordering::SeqCst);
        assert!(client.acquire_jobs(9, &f.session(), &[1]).await.is_err());
        assert_eq!(f.script.effects.load(Ordering::SeqCst), 1);
        f.script.groups_status.store(403, Ordering::SeqCst);
        assert_new_effects_blocked(&f, &client).await;
        assert_eq!(
            f.reads(),
            2,
            "acquisition {status} invalidates cached authority"
        );
        assert_eq!(
            f.script.effects.load(Ordering::SeqCst),
            1,
            "no effect after reproof refusal"
        );
    }
}

#[tokio::test]
async fn every_queue_response_and_retried_request_invalidates() {
    for status in [401, 403, 404, 429] {
        for ack in [false, true] {
            let f = Fixture::start().await;
            let client = f.client(false, true);
            client.ensure_route_proof().await.unwrap();
            f.script.queue_status.store(status, Ordering::SeqCst);
            if ack {
                let _ = client.ack_message(&f.session(), 1).await;
            } else {
                let _ = client.poll_messages(&f.session(), 0, 1).await;
            }
            client.ensure_route_proof().await.unwrap();
            assert_eq!(f.reads(), 2, "queue status {status}, ack={ack}");
        }
    }
    let f = Fixture::start().await;
    let client = f.client(false, true);
    client.ensure_route_proof().await.unwrap();
    f.script.groups_status.store(401, Ordering::SeqCst);
    f.script.groups_retry_status.store(404, Ordering::SeqCst);
    let before = client.proof_epoch.load(Ordering::SeqCst);
    assert!(client.probe_actions_access().await.is_err());
    assert_eq!(
        client.proof_epoch.load(Ordering::SeqCst),
        before + 2,
        "both the expired response and retried 404 invalidate"
    );
}

#[tokio::test]
async fn metadata_and_bootstrap_failures_invalidate_and_keep_retry_after() {
    let f = Fixture::start().await;
    let client = f.client(false, true);
    client.ensure_route_proof().await.unwrap();
    for token_failure in [false, true] {
        let status = if token_failure {
            &f.script.token_status
        } else {
            &f.script.metadata_status
        };
        status.store(429, Ordering::SeqCst);
        let error = client
            .resolve_target_identity(&client.config.target)
            .await
            .unwrap_err();
        assert!(
            matches!(error, AccessFailure::RateLimited { retry_after: Some(d) } if d.as_secs() == 7)
        );
        status.store(if token_failure { 201 } else { 200 }, Ordering::SeqCst);
        let before = f.reads();
        client.ensure_route_proof().await.unwrap();
        assert_eq!(f.reads(), before + 1);
    }
    // Admin bootstrap is retried after a 401, and its GitHub denial must
    // propagate without restoring the old proof.
    f.script.groups_status.store(401, Ordering::SeqCst);
    f.script.registration_status.store(429, Ordering::SeqCst);
    let err = client
        .probe_actions_access()
        .await
        .unwrap_err()
        .to_access_failure();
    assert!(matches!(err, AccessFailure::RateLimited { retry_after: Some(d) } if d.as_secs() == 7));
    let before = f.reads();
    let err = client.ensure_route_proof().await.unwrap_err();
    assert!(matches!(err, AccessFailure::RateLimited { retry_after: Some(d) } if d.as_secs() == 7));
    let cached = client.ensure_route_proof().await.unwrap_err();
    assert!(
        matches!(cached, AccessFailure::RateLimited { retry_after: Some(d) } if d.as_secs() == 7)
    );
    assert_eq!(f.reads(), before + 1);
}

async fn assert_new_effects_blocked(f: &Fixture, client: &crate::ScalesetClient) {
    let identity = ScaleSetIdentity {
        target: client.config.target.clone(),
        runner_group: "Default".into(),
        scale_set_name: "test".into(),
    };
    assert!(client.acquire_jobs(9, &f.session(), &[1]).await.is_err());
    assert!(client.generate_jit(9, "runner").await.is_err());
    assert!(client.establish_session(9, "owner").await.is_err());
    assert!(client.create_scale_set(&identity, 7, &[]).await.is_err());
    assert!(client.update_scale_set_labels(9, &[]).await.is_err());
}

#[tokio::test]
async fn admin_401_cannot_retry_a_new_management_effect_without_reproof() {
    for effect in ["create", "session", "jit", "labels"] {
        let f = Fixture::start().await;
        let client = f.client(false, true);
        client.ensure_route_proof().await.unwrap();
        f.script.effect_status.store(401, Ordering::SeqCst);
        f.script.groups_after_effect.store(403, Ordering::SeqCst);
        let identity = ScaleSetIdentity {
            target: client.config.target.clone(),
            runner_group: "Default".into(),
            scale_set_name: "test".into(),
        };
        let error = match effect {
            "create" => client
                .create_scale_set(&identity, 7, &[])
                .await
                .unwrap_err(),
            "session" => client.establish_session(9, "owner").await.unwrap_err(),
            "jit" => client.generate_jit(9, "runner").await.unwrap_err(),
            "labels" => client.update_scale_set_labels(9, &[]).await.unwrap_err(),
            _ => unreachable!(),
        };
        assert!(matches!(error, AccessFailure::PermissionDenied));
        assert_eq!(f.reads(), 2, "{effect}: the retry requires full reproof");
        assert_eq!(
            f.script.effects.load(Ordering::SeqCst),
            1,
            "{effect}: no dispatch after the denial"
        );
    }
}
