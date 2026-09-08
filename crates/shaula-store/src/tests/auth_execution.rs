//! Real SQLite regressions for captured authority and retained execution.
#![allow(clippy::unwrap_used)] // Assertions use fixed, non-secret test fixtures.
use super::auth_v2::{
    binding, org_selector, policy_json, promotion, seed_fleet, seed_v2_candidate, PROFILE,
};
use sea_orm::ConnectionTrait;
use shaula_core::auth_context::ResolvedAuthContext;
use shaula_core::registry::{AuthHandoffExpectation, FleetContextAck};

async fn execute(store: &crate::Store, sql: &str) {
    store.connection().execute_unprepared(sql).await.unwrap();
}

pub(super) async fn ready() -> (crate::Store, AuthHandoffExpectation, ResolvedAuthContext) {
    let store = super::profiles::store().await;
    seed_v2_candidate(&store, 1, &policy_json(&[&org_selector("example-org")])).await;
    store
        .auth_apply_full(
            PROFILE,
            1,
            true,
            None,
            5,
            Some(promotion(vec![binding("example-org", 100, 11)], 1, &[])),
        )
        .await
        .unwrap();
    seed_fleet(
        &store,
        "fleet",
        1,
        r#"{"kind":"organization","owner":"example-org"}"#,
    )
    .await;
    let desired = store
        .fleet_auth_context_get("fleet")
        .await
        .unwrap()
        .unwrap()
        .desired_context_json;
    let context = serde_json::from_str(desired.as_ref().unwrap()).unwrap();
    (
        store,
        AuthHandoffExpectation {
            mutation_fence: 1,
            desired_context_json: desired,
        },
        context,
    )
}

#[tokio::test]
async fn refreshed_equal_intent_cannot_acknowledge_an_old_network_attempt() {
    let (store, captured, context) = ready().await;
    execute(
        &store,
        "UPDATE fleets SET mutation_fence=2 WHERE key='fleet'",
    )
    .await;
    let tx = store.begin().await.unwrap();
    store
        .fleet_auth_context_set_desired_tx(
            &tx,
            "fleet",
            PROFILE,
            1,
            captured.desired_context_json.as_deref(),
            10,
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let json = serde_json::to_string(&context).unwrap();
    assert_eq!(
        store
            .handoff_acknowledge("fleet", PROFILE, 1, Some(&json), &captured)
            .await
            .unwrap(),
        FleetContextAck::Stale
    );
    assert!(store
        .handoff_get("fleet")
        .await
        .unwrap()
        .unwrap()
        .observed_revision
        .is_none());
    let fresh = AuthHandoffExpectation {
        mutation_fence: 2,
        ..captured
    };
    assert_eq!(
        store
            .handoff_acknowledge("fleet", PROFILE, 1, Some(&json), &fresh)
            .await
            .unwrap(),
        FleetContextAck::Acknowledged
    );
}

#[tokio::test]
async fn changed_intent_and_incomplete_or_drifted_authority_never_acknowledge() {
    let (store, captured, context) = ready().await;
    for changed in [
        ResolvedAuthContext {
            app_id: "another-app".into(),
            ..context.clone()
        },
        ResolvedAuthContext {
            github_host: "another.host".into(),
            ..context.clone()
        },
        ResolvedAuthContext {
            organization_id: None,
            ..context.clone()
        },
        ResolvedAuthContext {
            account_kind: shaula_core::auth_policy::AccountKind::User,
            ..context.clone()
        },
    ] {
        let json = serde_json::to_string(&changed).unwrap();
        assert_eq!(
            store
                .handoff_acknowledge("fleet", PROFILE, 1, Some(&json), &captured)
                .await
                .unwrap(),
            FleetContextAck::IdentityDrift
        );
    }
    let mut modified = context.clone();
    modified.login = "renamed-org".into();
    let tx = store.begin().await.unwrap();
    store
        .fleet_auth_context_set_desired_tx(
            &tx,
            "fleet",
            PROFILE,
            1,
            Some(&serde_json::to_string(&modified).unwrap()),
            10,
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        store
            .handoff_acknowledge(
                "fleet",
                PROFILE,
                1,
                Some(&serde_json::to_string(&context).unwrap()),
                &captured
            )
            .await
            .unwrap(),
        FleetContextAck::Stale
    );
    assert!(store
        .handoff_get("fleet")
        .await
        .unwrap()
        .unwrap()
        .observed_revision
        .is_none());
}

#[tokio::test]
async fn first_repository_resolution_cannot_overwrite_an_admission_pin() {
    let (store, _, mut context) = ready().await;
    context.target =
        shaula_core::github::GitHubTarget::new_repository("example-org", "repo").unwrap();
    context.organization_id = None;
    context.repository_id = Some(700);
    context.repository_owner_id = Some(100);
    let json = serde_json::to_string(&context).unwrap();
    let tx = store.begin().await.unwrap();
    store
        .fleet_auth_context_set_desired_tx(&tx, "fleet", PROFILE, 1, Some(&json), 10)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let captured = AuthHandoffExpectation {
        mutation_fence: 1,
        desired_context_json: Some(json),
    };
    context.repository_id = Some(701);
    assert_eq!(
        store
            .handoff_acknowledge(
                "fleet",
                PROFILE,
                1,
                Some(&serde_json::to_string(&context).unwrap()),
                &captured
            )
            .await
            .unwrap(),
        FleetContextAck::IdentityDrift
    );
    assert!(store
        .auth_execution_context_get("fleet", PROFILE, 1)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn historical_generation_and_session_references_survive_cross_profile_handoff() {
    let (store, captured, context) = ready().await;
    store
        .handoff_acknowledge(
            "fleet",
            PROFILE,
            1,
            Some(&serde_json::to_string(&context).unwrap()),
            &captured,
        )
        .await
        .unwrap();
    store
        .session_install("fleet", "session-old", 3, 10)
        .await
        .unwrap();
    store
        .generation_insert(shaula_core::registry::GenerationRecord {
            id: "generation".into(),
            fleet_key: "fleet".into(),
            runner_name: "runner".into(),
            generation_name: "generation".into(),
            fleet_revision: 1,
            template_profile_key: "template".into(),
            template_revision: 1,
            template_artifact_digest: "sha256:a".into(),
            attestation_id: "att".into(),
            inputs_digest: "sha256:b".into(),
            state: shaula_core::lifecycle::GenerationState::CreatePending,
            github_runner_id: Some(7),
            workspace_path: "work".into(),
            created_at: 10,
            updated_at: 10,
        })
        .await
        .unwrap();
    store
        .operation_insert(shaula_core::registry::OperationInsert {
            id: "jit-generation".into(),
            generation_id: "generation".into(),
            kind: "JitStarting".into(),
            state: "Pending".into(),
            provenance_json: Some(
                serde_json::json!({"context":{"auth_profile_key":PROFILE,"auth_revision":1}})
                    .to_string(),
            ),
            saved_plan_path: None,
            saved_plan_digest: None,
            now: 10,
        })
        .await
        .unwrap();
    // Replace desired and observed authority; the historical execution keeps
    // its original target even when the latest spec names a different one.
    execute(&store, "INSERT INTO github_auth_profiles(key,incarnation,desired_revision,active_revision,status,deletion_requested,created_at,updated_at)
        VALUES('other','other',1,1,'Active',0,1,1);
        INSERT INTO github_auth_profile_revisions(profile_key,revision,kind,app_id,installation_id,allowlist_json,credential_bytes,state,created_at,schema_version)
        VALUES('other',1,'github_app','1',1,'[]',X'01','Active',1,1);
        UPDATE fleet_revisions SET auth_desired_profile_key='other',spec_json='{\"github\":{\"target\":{\"kind\":\"organization\",\"owner\":\"other-org\"}}}' WHERE fleet_key='fleet';
        UPDATE fleet_auth_handoffs SET desired_profile_key='other',observed_profile_key='other',desired_revision=1,observed_revision=1 WHERE fleet_key='fleet';").await;
    let dependents = store.auth_live_dependents(PROFILE).await.unwrap();
    assert_eq!(dependents.len(), 1);
    assert_eq!(dependents[0].retained_contexts, vec![context.clone()]);
    assert!(dependents[0].target_json.contains("example-org"));
    assert_eq!(
        store.auth_generation_ref("generation").await.unwrap(),
        Some((PROFILE.into(), 1))
    );
    let before = shaula_core::registry::auth_dependent_set_fingerprint(&dependents);
    execute(
        &store,
        "UPDATE runner_generations SET state='Destroyed' WHERE id='generation'",
    )
    .await;
    assert_eq!(
        store.auth_live_dependents(PROFILE).await.unwrap().len(),
        1,
        "session still retains the route"
    );
    execute(&store, "DELETE FROM fleet_sessions WHERE fleet_key='fleet'").await;
    execute(
        &store,
        "UPDATE runner_operations SET state='ApplyStarting' WHERE generation_id='generation'",
    )
    .await;
    assert_eq!(
        store
            .operations_open_for_generation("generation")
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store.auth_live_dependents(PROFILE).await.unwrap().len(),
        1,
        "a pending operation can outlive the generation state"
    );
    execute(
        &store,
        "UPDATE runner_operations SET state='Succeeded' WHERE generation_id='generation'",
    )
    .await;
    let after = store.auth_live_dependents(PROFILE).await.unwrap();
    assert!(
        after.is_empty(),
        "unused history and terminal operations do not block"
    );
    assert_ne!(
        before,
        shaula_core::registry::auth_dependent_set_fingerprint(&after)
    );
    assert_eq!(
        store
            .auth_execution_context_get("fleet", PROFILE, 1)
            .await
            .unwrap(),
        Some(context)
    );
}

#[tokio::test]
async fn promotion_requires_every_dependent_in_checked_snapshot() {
    let (store, _, _) = ready().await;
    seed_v2_candidate(&store, 2, &policy_json(&[&org_selector("example-org")])).await;
    let deps = store.auth_live_dependents(PROFILE).await.unwrap();
    let mut facts = promotion(vec![binding("example-org", 100, 11)], 2, &deps);
    let mut snapshot: shaula_core::registry::AuthValidationSnapshot =
        serde_json::from_str(&facts.snapshot_json).unwrap();
    snapshot.checked_fleets.clear();
    facts.snapshot_json = serde_json::to_string(&snapshot).unwrap();
    assert_eq!(
        store
            .auth_apply_full(PROFILE, 2, true, None, 20, Some(facts))
            .await
            .unwrap(),
        shaula_core::registry::AuthPromotionOutcome::Restaged
    );
}
