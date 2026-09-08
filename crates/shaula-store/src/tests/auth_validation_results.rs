//! Completed validation outcomes are immutable, and transaction-time
//! rejection reasons reach both the Candidate and its public Change.

use super::auth_v2::{
    binding, org_selector, policy_json, promotion, seed_fleet, seed_v2_candidate, PROFILE,
};
use super::profiles::store;
use shaula_core::registry::{AuthPromotionOutcome, ProfileChangeInsert};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

async fn seed_change(store: &crate::Store, revision: i64) -> crate::StoreResult<String> {
    let id = format!("auth-change-{revision}");
    let tx = store.begin().await?;
    store
        .profile_change_insert(
            &tx,
            ProfileChangeInsert {
                id: id.clone(),
                resource_kind: "github_auth_profile".into(),
                profile_key: PROFILE.into(),
                revision: Some(revision),
                kind: "publish".into(),
                now: revision,
            },
        )
        .await?;
    tx.commit().await?;
    Ok(id)
}

#[tokio::test]
async fn late_rejection_cannot_rewrite_an_active_revision_or_change() -> TestResult {
    let store = store().await;
    seed_v2_candidate(&store, 1, &policy_json(&[&org_selector("example-org")])).await;
    let change_id = seed_change(&store, 1).await?;
    store
        .auth_apply_full(
            PROFILE,
            1,
            true,
            None,
            5,
            Some(promotion(vec![binding("example-org", 100, 11)], 1, &[])),
        )
        .await?;
    let before = store
        .auth_revision_get(PROFILE, 1)
        .await?
        .ok_or("revision missing")?;
    let change_before = store
        .profile_change_get(&change_id)
        .await?
        .ok_or("change missing")?;

    let result = store
        .auth_apply_full(PROFILE, 1, false, Some("PermissionDenied"), 9, None)
        .await;
    assert!(matches!(result, Err(crate::StoreError::Conflict { .. })));
    let after = store
        .auth_revision_get(PROFILE, 1)
        .await?
        .ok_or("revision missing")?;
    assert_eq!(after, before);
    assert_eq!(after.state, "Active");
    assert_eq!(
        store
            .auth_profile_get(PROFILE)
            .await?
            .ok_or("profile missing")?
            .active_revision,
        Some(1)
    );
    assert_eq!(
        store
            .profile_change_get(&change_id)
            .await?
            .ok_or("change missing")?,
        change_before
    );
    Ok(())
}

#[tokio::test]
async fn repeated_rejection_preserves_the_original_validation_evidence() -> TestResult {
    let store = store().await;
    seed_v2_candidate(&store, 1, &policy_json(&[&org_selector("example-org")])).await;
    let change_id = seed_change(&store, 1).await?;
    store
        .auth_apply_full(PROFILE, 1, false, Some("Unauthenticated"), 5, None)
        .await?;
    let result = store
        .auth_apply_full(PROFILE, 1, false, Some("PermissionDenied"), 9, None)
        .await;
    assert!(matches!(result, Err(crate::StoreError::Conflict { .. })));
    let row = store
        .auth_revision_get(PROFILE, 1)
        .await?
        .ok_or("revision missing")?;
    let change = store
        .profile_change_get(&change_id)
        .await?
        .ok_or("change missing")?;
    assert_eq!(row.reason.as_deref(), Some("Unauthenticated"));
    assert_eq!(change.reason, row.reason);
    assert_eq!(change.updated_at, 5);
    Ok(())
}

#[tokio::test]
async fn promotion_time_shrink_race_preserves_change_rejection_reason() -> TestResult {
    let store = store().await;
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
            Some(promotion(
                vec![
                    binding("example-org", 100, 11),
                    binding("other-org", 200, 22),
                ],
                1,
                &[],
            )),
        )
        .await?;
    seed_v2_candidate(&store, 2, &policy_json(&[&org_selector("other-org")])).await;
    let change_id = seed_change(&store, 2).await?;
    let checked = store.auth_live_dependents(PROFILE).await?;
    assert!(checked.is_empty());
    // Admission on the existing active policy races after the validator's
    // dependency snapshot but before Candidate promotion commits.
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
            Some(promotion(vec![binding("other-org", 200, 22)], 2, &checked)),
        )
        .await?;
    assert_eq!(outcome, AuthPromotionOutcome::Rejected);
    let row = store
        .auth_revision_get(PROFILE, 2)
        .await?
        .ok_or("revision missing")?;
    let change = store
        .profile_change_get(&change_id)
        .await?
        .ok_or("change missing")?;
    assert_eq!(row.state, "Rejected");
    assert_eq!(row.reason.as_deref(), Some("TargetPolicyInUse"));
    assert_eq!(change.state, "Rejected");
    assert_eq!(change.reason, row.reason);
    assert_eq!(change.updated_at, 9);
    assert_eq!(
        store
            .auth_profile_get(PROFILE)
            .await?
            .ok_or("profile missing")?
            .active_revision,
        Some(1)
    );
    assert!(store.auth_bindings_get(PROFILE, 2).await?.is_empty());
    Ok(())
}
