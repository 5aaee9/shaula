//! v2 multi-account promotion and exact-context persistence tests
//! (spec 0011 §4.1/§5.1/§5.2), split to keep files within 400 lines.

#![allow(clippy::unwrap_used)]

use super::profiles::store;
use shaula_core::auth_context::{AccountBinding, RepositorySelection};
use shaula_core::auth_policy::AccountKind;
use shaula_core::registry::{
    auth_dependent_set_fingerprint, AuthCheckedFleet, AuthDependentTarget, AuthPromotion,
    AuthPromotionOutcome, AuthValidationSnapshot,
};

pub(super) const PROFILE: &str = "shared-github";

pub(super) fn policy_json(selectors: &[&str]) -> String {
    format!(
        r#"{{"selectors":[{selectors}]}}"#,
        selectors = selectors.join(",")
    )
}

pub(super) fn org_selector(owner: &str) -> String {
    format!(r#"{{"kind":"organization","owner":"{owner}"}}"#)
}

pub(super) fn binding(login: &str, account_id: i64, installation_id: i64) -> AccountBinding {
    AccountBinding {
        account_id,
        account_kind: AccountKind::Organization,
        login: login.into(),
        installation_id,
        repository_selection: RepositorySelection::All,
        validated_at_ms: 4,
    }
}

pub(super) fn snapshot_for(revision: i64, dependents: &[AuthDependentTarget]) -> String {
    serde_json::to_string(&AuthValidationSnapshot {
        candidate: (PROFILE.to_string(), revision),
        dependent_set: auth_dependent_set_fingerprint(dependents),
        checked_fleets: dependents
            .iter()
            .map(|d| AuthCheckedFleet {
                key: d.fleet_key.clone(),
                incarnation: d.incarnation.clone(),
                revision: d.revision,
                fence: d.fence,
            })
            .collect(),
        identities: Vec::new(),
    })
    .unwrap()
}

pub(super) fn promotion(
    bindings: Vec<AccountBinding>,
    revision: i64,
    dependents: &[AuthDependentTarget],
) -> AuthPromotion {
    AuthPromotion {
        bindings,
        snapshot_json: snapshot_for(revision, dependents),
    }
}

/// Seeds a v2 Candidate revision and advances the desired head.
pub(super) async fn seed_v2_candidate(store: &crate::Store, revision: i64, policy: &str) {
    let tx = store.begin().await.unwrap();
    store
        .auth_commit_revision(
            &tx,
            shaula_core::auth::AuthRevisionInsert {
                key: PROFILE.into(),
                incarnation: "inc-v2".into(),
                revision,
                kind: "github_app".into(),
                app_id: Some("4863460".into()),
                installation_id: None,
                pat_principal: None,
                allowlist_json: String::new(),
                schema_version: 2,
                policy_json: Some(policy.to_string()),
            },
            b"-----BEGIN PRIVATE KEY-----",
            revision,
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

/// Seeds a fleet whose latest revision desires the profile at `revision`
/// with a concrete github target.
pub(super) async fn seed_fleet(store: &crate::Store, fleet: &str, revision: i64, target: &str) {
    let tx = store.begin().await.unwrap();
    store
        .fleet_commit_revision(
            &tx,
            shaula_core::fleet::FleetRevisionInsert {
                key: fleet.into(),
                incarnation: format!("inc-{fleet}"),
                revision: 1,
                spec_json: format!(r#"{{"github":{{"target":{target}}}}}"#),
                template: None,
                auth_desired: (PROFILE.into(), revision),
                inputs_digest: "sha256:inputs".into(),
                actor: "op".into(),
                now: revision,
            },
        )
        .await
        .unwrap();
    store
        .handoff_set_desired(&tx, fleet, PROFILE, revision)
        .await
        .unwrap();
    // Mirror the real fleet commit: the desired Resolved Auth Context is
    // derived from the ACTIVE revision's frozen bindings in the same
    // transaction (spec 0011 §4.2).
    store
        .fleet_auth_context_commit_tx(
            &tx,
            fleet,
            PROFILE,
            revision,
            &format!(r#"{{"github":{{"target":{target}}}}}"#),
            revision,
        )
        .await
        .unwrap()
        .unwrap();
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn v2_promotion_freezes_bindings_and_snapshot_atomically() {
    let store = store().await;
    seed_v2_candidate(&store, 1, &policy_json(&[&org_selector("example-org")])).await;
    // Promotion with an empty live dependent set: coverage passes
    // trivially and the bindings freeze atomically with the head advance.
    let bindings = vec![binding("example-org", 100, 11)];
    let outcome = store
        .auth_apply_full(
            PROFILE,
            1,
            true,
            None,
            9,
            Some(promotion(bindings.clone(), 1, &[])),
        )
        .await
        .unwrap();
    assert_eq!(outcome, AuthPromotionOutcome::Promoted);

    let head = store.auth_profile_get(PROFILE).await.unwrap().unwrap();
    assert_eq!(head.active_revision, Some(1));
    // Bindings frozen and readable per exact revision.
    let frozen = store.auth_bindings_get(PROFILE, 1).await.unwrap();
    assert_eq!(frozen, bindings);
    // Snapshot persisted on the revision row.
    let row = store.auth_revision_get(PROFILE, 1).await.unwrap().unwrap();
    assert_eq!(row.state, "Active");
    assert!(row.validation_snapshot_json.is_some());

    // A fleet admitted afterwards is part of the live dependent set and
    // its handoff points at the (now active) exact revision.
    seed_fleet(
        &store,
        "fleet-a",
        1,
        r#"{"kind":"organization","owner":"example-org"}"#,
    )
    .await;
    let dependents = store.auth_live_dependents(PROFILE).await.unwrap();
    assert_eq!(dependents.len(), 1);
    assert_eq!(dependents[0].fleet_key, "fleet-a");
    assert!(dependents[0].target_json.contains("example-org"));
    let handoff = store.handoff_get("fleet-a").await.unwrap().unwrap();
    assert_eq!(handoff.desired_profile_key, PROFILE);
    assert_eq!(handoff.desired_revision, 1);
}

#[tokio::test]
async fn v2_shrink_rejected_when_live_target_loses_coverage() {
    let store = store().await;
    // Revision 1 (active) covers example-org AND other-org; a fleet lives
    // on example-org. Revision 2 shrinks to other-org only.
    seed_v2_candidate(
        &store,
        1,
        &policy_json(&[&org_selector("example-org"), &org_selector("other-org")]),
    )
    .await;
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

    seed_v2_candidate(&store, 2, &policy_json(&[&org_selector("other-org")])).await;
    // The validator's snapshot covers only other-org; the in-transaction
    // coverage gate finds fleet-a live on example-org and REFUSES.
    let bindings = vec![binding("Other-Org", 300, 33)];
    let outcome = store
        .auth_apply_full(PROFILE, 2, true, None, 9, Some(promotion(bindings, 2, &[])))
        .await
        .unwrap();
    assert_eq!(outcome, AuthPromotionOutcome::Rejected);

    // The Candidate is rejected with TargetPolicyInUse; the prior active
    // revision keeps serving.
    let row = store.auth_revision_get(PROFILE, 2).await.unwrap().unwrap();
    assert_eq!(row.state, "Rejected");
    assert_eq!(row.reason.as_deref(), Some("TargetPolicyInUse"));
    let head = store.auth_profile_get(PROFILE).await.unwrap().unwrap();
    assert_eq!(head.active_revision, Some(1));
}

#[tokio::test]
async fn v2_dependent_set_change_under_validation_restages() {
    let store = store().await;
    // Revision 1 is active with a policy covering example-org.
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

    // Revision 2 is staged for validation. The validator checked a
    // dependent set of ZERO live fleets...
    seed_v2_candidate(
        &store,
        2,
        &policy_json(&[&org_selector("example-org"), &org_selector("extra-org")]),
    )
    .await;

    // ...then a fleet appears while the Candidate is in validation. Real
    // admission pins the fleet to the ACTIVE revision (1) — the Candidate
    // only takes over after promotion retargets it.
    seed_fleet(
        &store,
        "fleet-late",
        1,
        r#"{"kind":"organization","owner":"example-org"}"#,
    )
    .await;

    let outcome = store
        .auth_apply_full(
            PROFILE,
            2,
            true,
            None,
            9,
            Some(promotion(vec![binding("example-org", 100, 11)], 1, &[])),
        )
        .await
        .unwrap();
    assert_eq!(
        outcome,
        AuthPromotionOutcome::Restaged,
        "stale checks never promote"
    );
    let head = store.auth_profile_get(PROFILE).await.unwrap().unwrap();
    assert_eq!(head.active_revision, Some(1));

    // Revalidation with the fresh dependent-set fingerprint promotes.
    let dependents = store.auth_live_dependents(PROFILE).await.unwrap();
    assert_eq!(dependents.len(), 1);
    let outcome = store
        .auth_apply_full(
            PROFILE,
            2,
            true,
            None,
            11,
            Some(promotion(
                vec![binding("example-org", 100, 11)],
                2,
                &dependents,
            )),
        )
        .await
        .unwrap();
    assert_eq!(outcome, AuthPromotionOutcome::Promoted);
    let head = store.auth_profile_get(PROFILE).await.unwrap().unwrap();
    assert_eq!(head.active_revision, Some(2));
}

// (context/ack tests moved to auth_v2_context.rs)
