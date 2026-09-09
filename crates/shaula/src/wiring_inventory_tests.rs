//! Global inventory can contain ordinary runners and unrelated Scale Sets.

use super::*;
use serde_json::{json, Value};

fn inventory(unknown_current_runner: bool) -> Value {
    let mut runners = vec![
        json!({"id":101,"name":"ordinary-missing-scale-set"}),
        json!({"id":102,"name":"ordinary-zero-scale-set","runnerScaleSetId":0}),
        json!({"id":103,"name":"another-scale-set","runnerScaleSetId":9}),
    ];
    if unknown_current_runner {
        runners.push(json!({"id":104,"name":"unknown-current-runner","runnerScaleSetId":42}));
    }
    json!({"count":runners.len(),"value":runners})
}

#[tokio::test]
async fn unrelated_global_inventory_allows_adoption_and_later_label_update() {
    let (plane, mut wiring, listener, clock, mock) = setup().await;
    *listener.inventory_response.lock().unwrap() = Some(inventory(false));
    assert_eq!(
        plane
            .control_plane
            .fleet_get(FLEET)
            .await
            .unwrap()
            .unwrap()
            .phase,
        "Pending"
    );
    ready(&plane, &mut wiring, &clock).await;
    let original = plane
        .control_plane
        .scale_set_get(FLEET)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(original.scale_set_id, Some(42));
    assert_eq!(original.owned_scale_set_id, Some(42));
    assert_eq!(listener.session_creates.load(Ordering::SeqCst), 1);
    assert_eq!(mock.scale_set_creates.load(Ordering::SeqCst), 0);

    let revision = labels_tests::commit_labels(&plane, &["linux", "arm64"], NOW + 5_000).await;
    tick(&mut wiring, &clock, NOW + 6_000).await;
    tick(&mut wiring, &clock, NOW + 7_000).await;
    let updated = plane.control_plane.fleet_get(FLEET).await.unwrap().unwrap();
    assert_eq!(updated.phase, "Ready", "{updated:?}");
    assert_eq!(updated.observed_revision, revision);
    assert_eq!(listener.label_updates.load(Ordering::SeqCst), 1);
    assert_eq!(
        listener.scale_set()["labels"],
        json!([{"name":"linux","type":"Customer"},{"name":"arm64","type":"Customer"}])
    );
    let binding = plane
        .control_plane
        .scale_set_get(FLEET)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(binding.scale_set_id, original.scale_set_id);
    assert_eq!(binding.owned_scale_set_id, original.owned_scale_set_id);
    assert_eq!(binding.state, "Adopted");
    assert_eq!(mock.scale_set_creates.load(Ordering::SeqCst), 0);
    wiring.tasks.shutdown().await;
}

#[tokio::test]
async fn mixed_global_inventory_still_blocks_unknown_runner_in_current_scale_set() {
    let (plane, mut wiring, listener, clock, mock) = setup().await;
    *listener.inventory_response.lock().unwrap() = Some(inventory(true));
    tick(&mut wiring, &clock, NOW).await;
    tick(&mut wiring, &clock, NOW + 1_000).await;
    let blocked = plane.control_plane.fleet_get(FLEET).await.unwrap().unwrap();
    assert_eq!(blocked.phase, "Degraded");
    assert_eq!(
        blocked.last_condition_reason.as_deref(),
        Some("UnknownRemoteRunner")
    );
    let binding = plane
        .control_plane
        .scale_set_get(FLEET)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(binding.scale_set_id, Some(42));
    assert_eq!(binding.owned_scale_set_id, None);
    assert_eq!(binding.state, "UnknownRemoteRunner");
    assert_eq!(listener.session_creates.load(Ordering::SeqCst), 0);
    assert_eq!(listener.label_updates.load(Ordering::SeqCst), 0);
    assert_eq!(mock.scale_set_creates.load(Ordering::SeqCst), 0);
    wiring.tasks.shutdown().await;
}
