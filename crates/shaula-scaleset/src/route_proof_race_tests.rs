use super::fixture::Fixture;
use shaula_core::ports::GitHubAccessPort;
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[tokio::test]
async fn concurrent_callers_share_a_single_refresh() {
    let f = Fixture::start().await;
    let client = Arc::new(f.client(false, true));
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let client = client.clone();
        tasks.push(tokio::spawn(
            async move { client.ensure_route_proof().await },
        ));
    }
    for task in tasks {
        task.await.unwrap().unwrap();
    }
    assert_eq!(f.reads(), 1);
}

#[tokio::test]
async fn stale_evidence_collected_over_the_window_is_refused() {
    let f = Fixture::start().await;
    let client = Arc::new(f.client(false, true));
    f.script.block_metadata_once.store(true, Ordering::SeqCst);
    let refresh = {
        let client = client.clone();
        tokio::spawn(async move { client.ensure_route_proof().await })
    };
    f.script.metadata_started.notified().await;
    f.advance(60_000);
    f.script.metadata_release.notify_one();
    assert!(
        refresh.await.unwrap().is_err(),
        "oldest evidence already expired when refresh ends"
    );
    assert_eq!(f.script.effects.load(Ordering::SeqCst), 0);
    let proof = client.ensure_route_proof().await.unwrap();
    assert_eq!(proof.checked_at_unix_ms, 1_060_000);
    assert_eq!(
        f.reads(),
        2,
        "next attempt collects new evidence instead of caching expired success"
    );
}

#[tokio::test]
async fn concurrent_failure_prevents_refresh_publication_without_blocking_other_routes() {
    let f = Fixture::start().await;
    let client = Arc::new(f.client(false, true));
    client.ensure_route_proof().await.unwrap();
    f.advance(60_000);
    f.script.block_metadata_once.store(true, Ordering::SeqCst);
    let refresh = {
        let client = client.clone();
        tokio::spawn(async move { client.ensure_route_proof().await })
    };
    f.script.metadata_started.notified().await;
    // Another task observes denial while the refresh owns its mutex.
    f.script.queue_status.store(403, Ordering::SeqCst);
    assert!(client.ack_message(&f.session(), 1).await.is_err());
    let other = Fixture::start().await;
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        other.client(false, true).ensure_route_proof(),
    )
    .await
    .unwrap()
    .unwrap();
    f.script.metadata_release.notify_one();
    assert!(
        refresh.await.unwrap().is_err(),
        "the later success cannot erase an observed denial"
    );
    client.ensure_route_proof().await.unwrap();
    assert_eq!(f.reads(), 3);
}

#[tokio::test]
async fn invalidation_after_cached_publication_is_not_lost_when_mutex_is_busy() {
    let f = Fixture::start().await;
    let client = f.client(false, true);
    client.ensure_route_proof().await.unwrap();
    // Hold the same mutex that a waiter/publication would own. The HTTP
    // invalidation must work without acquiring it or clearing the entry.
    let state = client.proof.lock().await;
    f.script.queue_status.store(404, Ordering::SeqCst);
    assert!(client.ack_message(&f.session(), 1).await.is_err());
    drop(state);
    client.ensure_route_proof().await.unwrap();
    assert_eq!(
        f.reads(),
        2,
        "cached positives must match the current invalidation epoch"
    );
}

#[tokio::test]
async fn wall_clock_regression_cannot_extend_a_positive_proof() {
    let f = Fixture::start().await;
    let client = f.client(false, true);
    client.ensure_route_proof().await.unwrap();
    f.advance(-1);
    client.ensure_route_proof().await.unwrap();
    assert_eq!(f.reads(), 2, "evidence from the future cannot be reused");
}
