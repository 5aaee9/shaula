//! Organization-wide agent inventories include ordinary runners as well as Scale Sets.

use serde_json::{json, Value};
use shaula_core::ports::{AccessFailure, GitHubAccessPort, RunnerLookup};
use shaula_scaleset::client::ScalesetClient;

use super::{app_client, wire_mock_router};

async fn client_for(inventory: Value) -> ScalesetClient {
    app_client(wire_mock_router::spawn_mock_github_with_inventory(inventory).await)
}

#[tokio::test]
async fn inventory_returns_only_requested_scale_set_in_a_mixed_organization() {
    let client = client_for(json!({
        "count": 5,
        "value": [
            {"id": 11, "name": "blacksmith-ordinary"},
            {"id": 12, "name": "ordinary-zero", "runnerScaleSetId": 0},
            {"id": 13, "name": "another-scale-set", "runnerScaleSetId": 8},
            {"id": 14, "name": "shaula-generation-1", "runnerScaleSetId": 9},
            {"id": 15, "name": "shaula-generation-2", "runnerScaleSetId": 9}
        ]
    }))
    .await;

    let runners = client.list_runners(9).await.unwrap();
    assert_eq!(runners.len(), 2);
    assert_eq!(runners[0].id, 14);
    assert_eq!(runners[0].name, "shaula-generation-1");
    assert_eq!(runners[1].id, 15);
    assert_eq!(runners[1].name, "shaula-generation-2");
    assert!(runners.iter().all(|runner| runner.scale_set_id == 9));
}

#[tokio::test]
async fn ordinary_runners_do_not_block_an_empty_scale_set_inventory() {
    let client = client_for(json!({
        "count": 3,
        "value": [
            {"id": 11, "name": "blacksmith-ordinary"},
            {"id": 12, "name": "ordinary-zero", "runnerScaleSetId": 0},
            {"id": 13, "name": "another-scale-set", "runnerScaleSetId": 8}
        ]
    }))
    .await;

    assert!(client.list_runners(9).await.unwrap().is_empty());
}

#[tokio::test]
async fn malformed_inventory_still_cannot_prove_scale_set_absence() {
    for inventory in [
        json!({"count": 1, "value": []}),
        json!({"count": -1, "value": []}),
        json!({"count": 0, "value": [{"id": 11, "name": "ordinary"}]}),
        json!({"count": 1, "value": [{"id": 0, "name": "ordinary"}]}),
        json!({"count": 1, "value": [{"id": -1, "name": "ordinary"}]}),
        json!({"count": 1, "value": [{"id": 11, "name": ""}]}),
        json!({"count": 1, "value": [{"id": 11, "name": "bad", "runnerScaleSetId": -1}]}),
    ] {
        let client = client_for(inventory.clone()).await;
        assert!(
            matches!(
                client.list_runners(9).await,
                Err(AccessFailure::Unavailable { .. })
            ),
            "malformed inventory must fail closed: {inventory}"
        );
    }
}

#[tokio::test]
async fn exact_name_lookup_never_assigns_ordinary_runners_to_a_scale_set() {
    for runner in [
        json!({"id": 11, "name": "shaula-generation"}),
        json!({"id": 11, "name": "shaula-generation", "runnerScaleSetId": 0}),
    ] {
        let client = client_for(json!({"count": 1, "value": [runner]})).await;
        assert!(matches!(
            client.get_runner_by_name(9, "shaula-generation").await,
            Err(AccessFailure::Unavailable { .. })
        ));
    }
}

#[tokio::test]
async fn exact_name_lookup_requires_the_requested_name_and_scale_set() {
    let other_set = client_for(json!({
        "count": 1,
        "value": [{"id": 11, "name": "shaula-generation", "runnerScaleSetId": 8}]
    }))
    .await;
    assert!(matches!(
        other_set
            .get_runner_by_name(9, "shaula-generation")
            .await
            .unwrap(),
        RunnerLookup::None
    ));

    let wrong_name = client_for(json!({
        "count": 1,
        "value": [{"id": 12, "name": "another-generation", "runnerScaleSetId": 9}]
    }))
    .await;
    assert!(matches!(
        wrong_name.get_runner_by_name(9, "shaula-generation").await,
        Err(AccessFailure::Unavailable { .. })
    ));
}

#[tokio::test]
async fn nonpositive_requested_scale_set_ids_never_prove_membership_or_absence() {
    let client = client_for(json!({
        "count": 1,
        "value": [{"id": 11, "name": "ordinary"}]
    }))
    .await;
    for scale_set_id in [0, -1] {
        assert!(matches!(
            client.list_runners(scale_set_id).await,
            Err(AccessFailure::Unavailable { .. })
        ));
        assert!(matches!(
            client.get_runner_by_name(scale_set_id, "ordinary").await,
            Err(AccessFailure::Unavailable { .. })
        ));
    }
}

#[tokio::test]
async fn exact_name_lookup_preserves_ambiguous_matches() {
    let client = client_for(json!({
        "count": 2,
        "value": [
            {"id": 11, "name": "shaula-generation", "runnerScaleSetId": 9},
            {"id": 12, "name": "shaula-generation", "runnerScaleSetId": 9}
        ]
    }))
    .await;
    assert!(matches!(
        client
            .get_runner_by_name(9, "shaula-generation")
            .await
            .unwrap(),
        RunnerLookup::Multiple
    ));
}

#[tokio::test]
async fn a_complete_empty_inventory_can_prove_no_matching_runner() {
    let client = client_for(json!({"count": 0, "value": []})).await;
    assert!(client.list_runners(9).await.unwrap().is_empty());
    assert!(matches!(
        client
            .get_runner_by_name(9, "shaula-generation")
            .await
            .unwrap(),
        RunnerLookup::None
    ));
}
