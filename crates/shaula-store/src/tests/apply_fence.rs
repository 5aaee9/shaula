//! The fenced apply-start boundary and the generation state-identity
//! persistence (spec 0002 §8, 0004 §5).
#![allow(clippy::unwrap_used)]

use super::persistence::store;
#[tokio::test]
async fn apply_start_fence_distinguishes_create_from_destroy() {
    let store = store().await;
    super::persistence::seed_auth_profile(&store, "prod-app", "inc-1", 1).await;
    use shaula_core::lifecycle::GenerationState;
    use shaula_core::registry::GenerationRecord;

    // Fleet at head r1 + one generation created from that revision.
    let tx = store.begin().await.unwrap();
    store
        .fleet_commit_revision(
            &tx,
            shaula_core::fleet::FleetRevisionInsert {
                key: "fence-fleet".into(),
                incarnation: "inc-1".into(),
                revision: 1,
                spec_json: r#"{"key":"fence-fleet"}"#.into(),
                template: None,
                auth_desired: ("prod-app".into(), 1),
                inputs_digest: "sha256:inputs".into(),
                actor: "admin".into(),
                now: 1000,
            },
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    store
        .generation_insert(GenerationRecord {
            id: "gen-1".into(),
            fleet_key: "fence-fleet".into(),
            runner_name: "runner-1".into(),
            generation_name: "s1".into(),
            fleet_revision: 1,
            template_profile_key: "tpl".into(),
            template_revision: 1,
            template_artifact_digest: "sha256:artifact".into(),
            attestation_id: "att-1".into(),
            inputs_digest: "sha256:inputs".into(),
            state: GenerationState::Creating,
            github_runner_id: None,
            workspace_path: "ws".into(),
            created_at: 1000,
            updated_at: 1000,
        })
        .await
        .unwrap();

    let insert = |kind: &'static str| shaula_core::registry::OperationInsert {
        id: shaula_core::auth::new_attempt_id(),
        generation_id: "gen-1".into(),
        kind: kind.into(),
        state: "ApplyStarting".into(),
        provenance_json: Some("{}".into()),
        saved_plan_path: Some("tfplan".into()),
        saved_plan_digest: Some("sha256:plan".into()),
        now: 1100,
    };

    // Head unchanged: the Create claim records fine.
    let tx = store.begin().await.unwrap();
    let outcome = store
        .operation_apply_starting_tx(&tx, insert("Create"))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(
        outcome.is_ok(),
        "unchanged head must admit the Create claim"
    );

    // A committed DELETE bumps the revision and sets the marker — the
    // record and the fence check share ONE transaction, so the stale
    // Create is refused AT the durable boundary (F05).
    let tx = store.begin().await.unwrap();
    use crate::entities::fleet::fleets;
    use sea_orm::{ActiveValue::Set, EntityTrait};
    let fleet = fleets::Entity::find_by_id("fence-fleet".to_string())
        .one(&tx)
        .await
        .unwrap()
        .unwrap();
    let mut updated: fleets::ActiveModel = fleet.into();
    updated.desired_revision = Set(2);
    updated.deletion_marker = Set(true);
    fleets::Entity::update(updated).exec(&tx).await.unwrap();
    tx.commit().await.unwrap();

    let tx = store.begin().await.unwrap();
    let outcome = store
        .operation_apply_starting_tx(&tx, insert("Create"))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(
        outcome.is_err(),
        "a deleted fleet must refuse the stale Create claim"
    );

    // Destroy runs with the generation's ORIGINAL materials and must
    // NOT be blocked by the decommission (F02, spec 0002 §8.330).
    let tx = store.begin().await.unwrap();
    let outcome = store
        .operation_apply_starting_tx(&tx, insert("Destroy"))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(
        outcome.is_ok(),
        "destroy after DELETE must be admitted for cleanup"
    );

    // A missing generation is corrupt for BOTH intents.
    let mut missing = insert("Destroy");
    missing.generation_id = "gen-missing".into();
    let tx = store.begin().await.unwrap();
    let outcome = store
        .operation_apply_starting_tx(&tx, missing)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(
        outcome.is_err(),
        "missing generation must refuse the record"
    );
}

#[tokio::test]
async fn generation_state_identity_roundtrip_and_destroy_marker() {
    let store = store().await;
    use shaula_core::lifecycle::GenerationState;
    use shaula_core::registry::GenerationRecord;

    store
        .generation_insert(GenerationRecord {
            id: "gen-id-1".into(),
            fleet_key: "f".into(),
            runner_name: "r".into(),
            generation_name: "s".into(),
            fleet_revision: 1,
            template_profile_key: "tpl".into(),
            template_revision: 1,
            template_artifact_digest: "sha256:a".into(),
            attestation_id: "att".into(),
            inputs_digest: "sha256:i".into(),
            state: GenerationState::Creating,
            github_runner_id: None,
            workspace_path: "ws".into(),
            created_at: 1,
            updated_at: 1,
        })
        .await
        .unwrap();

    // No result yet: identity is None.
    assert!(store
        .generation_state_identity("gen-id-1")
        .await
        .unwrap()
        .is_none());
    assert!(!store
        .generation_destroy_attempted("gen-id-1")
        .await
        .unwrap());

    // The supervisor persists the wrapper with the REAL identity (F07).
    let body = serde_json::json!({
        "result": {"roles": {}},
        "state_lineage": "lineage-xyz",
        "state_serial": 9,
    })
    .to_string();
    store
        .generation_set_result("gen-id-1", &body, "sha256:result", 2)
        .await
        .unwrap();
    assert_eq!(
        store.generation_state_identity("gen-id-1").await.unwrap(),
        Some(("lineage-xyz".to_string(), 9))
    );
    // Recording a Destroy operation flips the retry discriminator.
    let tx = store.begin().await.unwrap();
    let outcome = store
        .operation_apply_starting_tx(
            &tx,
            shaula_core::registry::OperationInsert {
                id: "op-1".into(),
                generation_id: "gen-id-1".into(),
                kind: "Destroy".into(),
                state: "ApplyStarting".into(),
                provenance_json: None,
                saved_plan_path: None,
                saved_plan_digest: None,
                now: 3,
            },
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(outcome.is_ok());
    assert!(store
        .generation_destroy_attempted("gen-id-1")
        .await
        .unwrap());
}
