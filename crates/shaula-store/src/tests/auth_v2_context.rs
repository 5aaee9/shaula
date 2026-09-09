//! v2 exact-context persistence tests (spec 0011 §4.2/§5.2), split
//! from auth_v2.rs to keep files within the 400-line budget.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::auth_v2::{
    binding, org_selector, policy_json, promotion, seed_fleet, seed_v2_candidate, PROFILE,
};
use super::profiles::store;
use shaula_core::auth_context::ResolvedAuthContext;
use shaula_core::auth_policy::AccountKind;
use shaula_core::registry::{AuthPromotion, AuthPromotionOutcome, AuthRepoProof};

async fn expectation(
    store: &crate::Store,
    fleet: &str,
) -> shaula_core::registry::AuthHandoffExpectation {
    shaula_core::registry::AuthHandoffExpectation {
        mutation_fence: 1,
        desired_context_json: store
            .fleet_auth_context_get(fleet)
            .await
            .unwrap()
            .and_then(|row| row.desired_context_json),
    }
}
#[tokio::test]
async fn v2_context_ack_is_atomic_and_refuses_same_name_rebuild() {
    let store = store().await;
    seed_v2_candidate(
        &store,
        1,
        &policy_json(&[
            r#"{"kind":"account_repositories","account_kind":"user","owner":"5aaee9"}"#,
        ]),
    )
    .await;
    let mut account_binding = binding("5aaee9", 200, 22);
    account_binding.account_kind = AccountKind::User;
    store
        .auth_apply_full(
            PROFILE,
            1,
            true,
            None,
            5,
            Some(promotion(vec![account_binding], 1, &[])),
        )
        .await
        .unwrap();
    seed_fleet(
        &store,
        "fleet-repo",
        1,
        r#"{"kind":"repository","owner":"5aaee9","repository":"proj"}"#,
    )
    .await;

    // Seed the desired context exactly as the fleet commit would.
    let mut desired = ResolvedAuthContext {
        profile_key: PROFILE.into(),
        revision: 1,
        github_host: "github.com".into(),
        app_id: "4863460".into(),
        account_id: 200,
        account_kind: AccountKind::User,
        login: "5aaee9".into(),
        installation_id: 22,
        target: shaula_core::github::GitHubTarget::new_repository("5aaee9", "proj").unwrap(),
        organization_id: None,
        repository_id: None,
        repository_owner_id: None,
    };
    let tx = store.begin().await.unwrap();
    store
        .fleet_auth_context_set_desired_tx(
            &tx,
            "fleet-repo",
            PROFILE,
            1,
            serde_json::to_string(&desired).unwrap().as_str(),
            6,
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // First verified resolution pins the repository identity: ref AND
    // context advance.
    desired.repository_id = Some(100);
    desired.repository_owner_id = Some(200);
    let ack = store
        .handoff_acknowledge(
            "fleet-repo",
            PROFILE,
            1,
            Some(serde_json::to_string(&desired).unwrap().as_str()),
            &expectation(&store, "fleet-repo").await,
        )
        .await
        .unwrap();
    assert_eq!(ack, shaula_core::registry::FleetContextAck::Acknowledged);

    // A same-name rebuild with a DIFFERENT repository id is a durable
    // identity drift: nothing is overwritten.
    desired.repository_id = Some(999);
    let ack = store
        .handoff_acknowledge(
            "fleet-repo",
            PROFILE,
            1,
            Some(serde_json::to_string(&desired).unwrap().as_str()),
            &expectation(&store, "fleet-repo").await,
        )
        .await
        .unwrap();
    assert_eq!(ack, shaula_core::registry::FleetContextAck::IdentityDrift);
    let row = store
        .fleet_auth_context_get("fleet-repo")
        .await
        .unwrap()
        .unwrap();
    let observed: ResolvedAuthContext =
        serde_json::from_str(row.observed_context_json.as_deref().unwrap()).unwrap();
    assert_eq!(observed.repository_id, Some(100), "the pin stands");
}

async fn execute(store: &crate::Store, sql: &str) {
    use sea_orm::ConnectionTrait;
    store.connection().execute_unprepared(sql).await.unwrap();
}

/// R3/R8: a v2 rotation advances the handoff ref AND the fleet's desired
/// context together (the retarget writes both), the later acknowledgement
/// CAS-stales when the fleet's mutation fence moved under it, and an
/// explicitly promoted newer revision may REPLACE the installation route

#[tokio::test]
async fn v2_rotation_advances_context_and_ack_cas_on_fence() {
    let store = store().await;
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
        "fleet-a",
        1,
        r#"{"kind":"organization","owner":"example-org"}"#,
    )
    .await;

    // Rotation: revision 2 is validated and promoted. Its retarget must
    // advance BOTH the handoff tuple and the desired context.
    seed_v2_candidate(&store, 2, &policy_json(&[&org_selector("example-org")])).await;
    let dependents = store.auth_live_dependents(PROFILE).await.unwrap();
    let outcome = store
        .auth_apply_full(
            PROFILE,
            2,
            true,
            None,
            9,
            Some(promotion(
                vec![binding("example-org", 100, 12)],
                2,
                &dependents,
            )),
        )
        .await
        .unwrap();
    assert_eq!(outcome, AuthPromotionOutcome::Promoted);
    let handoff = store.handoff_get("fleet-a").await.unwrap().unwrap();
    assert_eq!(handoff.desired_revision, 2);
    let context = store
        .fleet_auth_context_get("fleet-a")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(context.desired_profile_key.as_deref(), Some(PROFILE));
    assert_eq!(context.desired_revision, Some(2));
    assert_eq!(context.state, "Pending");

    // Acknowledge the verified context for revision 2: the prior pin
    // (installation 11, revision 1) is REPLACED by the explicitly
    // promoted route (installation 12, revision 2) — the sanctioned
    // reinstall path (spec 0011 §5.1).
    let verified = ResolvedAuthContext {
        profile_key: PROFILE.into(),
        revision: 2,
        github_host: "github.com".into(),
        app_id: "4863460".into(),
        account_id: 100,
        account_kind: AccountKind::Organization,
        login: "example-org".into(),
        installation_id: 12,
        target: shaula_core::github::GitHubTarget::organization("example-org").unwrap(),
        organization_id: Some(100),
        repository_id: None,
        repository_owner_id: None,
    };
    let ack = store
        .handoff_acknowledge(
            "fleet-a",
            PROFILE,
            2,
            Some(serde_json::to_string(&verified).unwrap().as_str()),
            &expectation(&store, "fleet-a").await,
        )
        .await
        .unwrap();
    assert_eq!(ack, shaula_core::registry::FleetContextAck::Acknowledged);

    // Fence CAS (R3): a concurrent fleet PUT bumps the mutation fence
    // after the desired intent was captured; an in-flight acknowledgement
    // of that stale intent loses — neither record changes.
    execute(
        &store,
        "UPDATE fleets SET mutation_fence = mutation_fence + 1 WHERE key = 'fleet-a'",
    )
    .await;
    let ack = store
        .handoff_acknowledge(
            "fleet-a",
            PROFILE,
            2,
            Some(serde_json::to_string(&verified).unwrap().as_str()),
            &expectation(&store, "fleet-a").await,
        )
        .await
        .unwrap();
    assert_eq!(ack, shaula_core::registry::FleetContextAck::Stale);
    let handoff = store.handoff_get("fleet-a").await.unwrap().unwrap();
    assert_eq!(handoff.observed_revision, Some(2), "first ack stands");
}

/// F6/R5: the ACTIVE revision's proven exact-repository identity carries
/// into the FIRST target resolution — a repository recreated between
/// profile activation and its first Fleet cannot silently establish a
/// new pin (the later handoff ack will detect the drift).
#[tokio::test]
async fn admission_context_carries_exact_repo_proofs_from_snapshot() {
    use shaula_core::auth_context::ResolvedAuthContext;
    let store = store().await;
    seed_v2_candidate(
        &store,
        1,
        &policy_json(&[r#"{"kind":"repository","owner":"5aaee9","repository":"proj"}"#]),
    )
    .await;

    // Promotion with a proven exact-repository identity (id 700/55).
    let mut binding = binding("5aaee9", 200, 22);
    binding.account_kind = AccountKind::User;
    let promotion = AuthPromotion {
        bindings: vec![binding],
        snapshot_json: serde_json::to_string(&shaula_core::registry::AuthValidationSnapshot {
            candidate: (PROFILE.to_string(), 1),
            dependent_set: shaula_core::registry::auth_dependent_set_fingerprint(&[]),
            checked_fleets: Vec::new(),
            identities: vec![shaula_core::registry::AuthIdentityProof {
                login: "5aaee9".into(),
                account_id: 200,
                installation_id: 22,
                repositories: vec![AuthRepoProof {
                    owner: "5aaee9".into(),
                    repository: "proj".into(),
                    repository_id: 700,
                    owner_id: 200,
                }],
            }],
        })
        .unwrap(),
    };
    let outcome = store
        .auth_apply_full(PROFILE, 1, true, None, 5, Some(promotion))
        .await
        .unwrap();
    assert_eq!(outcome, AuthPromotionOutcome::Promoted);

    // Admission derives the desired context for the fleet target — the
    // proven repository id MUST carry over into the intent.
    let spec =
        r#"{"github":{"target":{"kind":"repository","owner":"5aaee9","repository":"proj"}}}"#;
    let tx = store.begin().await.unwrap();
    let context_json = store
        .auth_desired_context_tx(&tx, PROFILE, 1, spec)
        .await
        .expect("v2 admission derives a context");
    let context: ResolvedAuthContext = serde_json::from_str(&context_json).unwrap();
    assert_eq!(context.repository_id, Some(700));
    assert_eq!(context.repository_owner_id, Some(200));
}
