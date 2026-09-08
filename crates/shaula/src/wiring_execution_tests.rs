//! Exact execution-authority composition regressions.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;

#[tokio::test]
async fn ownership_reports_actual_proof_health_for_the_observed_revision() {
    use shaula_core::registry::AuthExecutionStore;
    use std::sync::atomic::Ordering;
    for (repository_id, healthy) in [(700, true), (999, false)] {
        let mock = crate::auth_worker_mock::mock_server(false).await;
        let plane = test_plane().await;
        let fence = seed_promoted_profile_and_fleet(&plane).await;
        acknowledge_handoff(&plane, fence).await;
        mock.repo_id.store(repository_id, Ordering::SeqCst);
        let mut wiring = wiring_with(
            &plane.control_plane,
            crate::auth_worker_mock::endpoints(&mock.base),
        )
        .await;
        let now = 1_800_000_000_000;
        tick_and_drain(&mut wiring, now).await;
        let observations = plane
            .control_plane
            .auth_route_observations(KEY, 1, now)
            .await
            .unwrap();
        assert_eq!(observations.len(), 1);
        let observation = &observations[0];
        assert_eq!(observation.fleet_key, FLEET);
        assert_eq!(observation.context.repository_id, Some(700));
        assert_eq!(observation.healthy, healthy);
        assert_eq!(observation.reason.is_none(), healthy);
        if !healthy {
            assert_eq!(mock.scale_set_creates.load(Ordering::SeqCst), 0);
        }
        assert!(plane
            .control_plane
            .auth_route_observations(KEY, 1, now + 60_001)
            .await
            .unwrap()
            .is_empty());
    }
}

// ---- G3 tests ---------------------------------------------------------------

/// G3: the execution supervisor is built FROM the persisted observed
/// authority — the handoff's observed ref plus the observed context row —
/// and only then is it eligible to run effects.
#[tokio::test]
async fn wiring_builds_execution_supervisor_from_observed_context() {
    let plane = test_plane().await;
    let fence = seed_promoted_profile_and_fleet(&plane).await;
    acknowledge_handoff(&plane, fence).await;
    let mut wiring = execution_wiring(&plane).await;
    let (_, fleet_revision, phase) =
        ControlPlaneStore::fleet_list(&*plane.control_plane, &fleet_actor())
            .await
            .unwrap()[0]
            .clone();
    let supervisor = wiring
        .supervisor_for(FLEET, fleet_revision, &phase)
        .await
        .unwrap();
    assert!(
        supervisor.is_some(),
        "a fully observed authority must yield an executable supervisor"
    );
}

/// G3: a CORRUPT observed context (durable crash/corruption state) must
/// never yield an executable port — the wiring refuses to build the
/// supervisor instead of running effects under an unreadable authority.
#[tokio::test]
async fn wiring_refuses_execution_supervisor_on_corrupt_observed_context() {
    let plane = test_plane().await;
    let fence = seed_promoted_profile_and_fleet(&plane).await;
    acknowledge_handoff(&plane, fence).await;
    corrupt_observed_context(&plane.db_path).await;
    let mut wiring = execution_wiring(&plane).await;
    let (_, fleet_revision, phase) =
        ControlPlaneStore::fleet_list(&*plane.control_plane, &fleet_actor())
            .await
            .unwrap()[0]
            .clone();
    let supervisor = wiring
        .supervisor_for(FLEET, fleet_revision, &phase)
        .await
        .unwrap();
    assert!(
        supervisor.is_none(),
        "a corrupt observed authority must fail closed: no supervisor, no effects"
    );
}

/// G3: while a rotation handoff is IN FLIGHT (observed rev 1, desired
/// rev 2), the execution authority stays the OBSERVED revision — the
/// supervisor still resolves instead of blocking on the Validating
/// candidate.
#[tokio::test]
async fn wiring_execution_authority_stays_observed_during_in_flight_rotation() {
    use std::sync::atomic::Ordering;
    let mock = crate::auth_worker_mock::mock_server(false).await;
    let plane = test_plane().await;
    let fence = seed_promoted_profile_and_fleet(&plane).await;
    acknowledge_handoff(&plane, fence).await;

    // A new candidate + its promotion rotates the fleet's desired tuple
    // to rev 2 while the handoff observed stays on rev 1.
    crate::auth_worker_v2::tests::seed_candidate(
        &plane.control_plane,
        2,
        r#"{"selectors":[{"kind":"repository","owner":"5aaee9","repository":"proj"}]}"#,
    )
    .await;
    let dependents = ControlPlaneStore::auth_live_dependents(&*plane.control_plane, KEY)
        .await
        .unwrap();
    assert_eq!(
        dependents.len(),
        1,
        "fixture: the fleet depends on the profile"
    );
    // The rotated revision freezes the SAME account binding (the account
    // did not change — only the revision advances).
    let mut snapshot: shaula_core::registry::AuthValidationSnapshot = serde_json::from_str(
        &crate::auth_worker_mock::promotion_from(KEY, 2, &dependents).snapshot_json,
    )
    .unwrap();
    let prior = plane
        .control_plane
        .auth_revision_get(KEY, 1)
        .await
        .unwrap()
        .unwrap();
    let mut prior_snapshot: shaula_core::registry::AuthValidationSnapshot =
        serde_json::from_str(&prior.validation_snapshot_json.unwrap()).unwrap();
    for proof in &mut prior_snapshot.identities {
        proof.installation_id = 23;
    }
    snapshot.identities = prior_snapshot.identities;
    let promotion = shaula_core::registry::AuthPromotion {
        bindings: vec![shaula_core::auth_context::AccountBinding {
            account_id: 220,
            account_kind: shaula_core::auth_policy::AccountKind::User,
            login: "5aaee9".into(),
            installation_id: 23,
            repository_selection: shaula_core::auth_context::RepositorySelection::Selected,
            validated_at_ms: 1,
        }],
        snapshot_json: serde_json::to_string(&snapshot).unwrap(),
    };
    let outcome = ControlPlaneStore::auth_apply_validation_v2(
        plane.control_plane.as_ref(),
        KEY,
        2,
        true,
        None,
        7,
        Some(promotion),
    )
    .await
    .unwrap();
    assert_eq!(
        outcome,
        shaula_core::registry::AuthPromotionOutcome::Promoted
    );

    let handoff = ControlPlaneStore::handoff_get(&*plane.control_plane, FLEET)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        handoff.desired,
        (KEY.to_string(), 2),
        "fixture: desired rotated"
    );
    assert_eq!(
        handoff.observed,
        Some((KEY.to_string(), 1)),
        "fixture: the handoff is in flight"
    );

    let mut wiring = wiring_with(
        &plane.control_plane,
        crate::auth_worker_mock::endpoints(&mock.base),
    )
    .await;
    let (_, fleet_revision, phase) =
        ControlPlaneStore::fleet_list(&*plane.control_plane, &fleet_actor())
            .await
            .unwrap()[0]
            .clone();
    let supervisor = wiring
        .supervisor_for(FLEET, fleet_revision, &phase)
        .await
        .unwrap();
    assert!(
        supervisor.is_some(),
        "the in-flight observed authority must remain available for handoff"
    );
    tick_and_drain(&mut wiring, 1_800_000_000_000).await;
    let row = plane
        .control_plane
        .fleet_auth_context_get(FLEET)
        .await
        .unwrap()
        .unwrap();
    let context: shaula_core::auth_context::ResolvedAuthContext =
        serde_json::from_str(&row.observed_context_json.unwrap()).unwrap();
    assert_eq!(context.revision, 2);
    assert_eq!(context.installation_id, 23);
    assert_eq!(&*mock.installation_reads.lock().unwrap(), &[23]);
    assert_eq!(mock.scale_set_creates.load(Ordering::SeqCst), 0);
    tick_and_drain(&mut wiring, 1_800_000_001_000).await;
    assert_eq!(mock.scale_set_creates.load(Ordering::SeqCst), 1);
    assert!(mock
        .installation_reads
        .lock()
        .unwrap()
        .iter()
        .all(|id| *id == 23));
}

#[tokio::test]
async fn handoff_rebuilds_bound_client_before_first_effect() {
    use std::sync::atomic::Ordering;
    let mock = crate::auth_worker_mock::mock_server(false).await;
    let plane = test_plane().await;
    seed_promoted_profile_and_fleet(&plane).await;
    let mut wiring = wiring_with(
        &plane.control_plane,
        crate::auth_worker_mock::endpoints(&mock.base),
    )
    .await;
    tick_and_drain(&mut wiring, 1_800_000_000_000).await;
    let row = plane
        .control_plane
        .fleet_auth_context_get(FLEET)
        .await
        .unwrap()
        .unwrap();
    let observed: shaula_core::auth_context::ResolvedAuthContext =
        serde_json::from_str(&row.observed_context_json.unwrap()).unwrap();
    assert_eq!(observed.repository_id, Some(700));
    assert_eq!(mock.scale_set_creates.load(Ordering::SeqCst), 0);
    tick_and_drain(&mut wiring, 1_800_000_001_000).await;
    assert_eq!(mock.scale_set_creates.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn exact_admission_pin_rejects_recreation_before_first_handoff() {
    use std::sync::atomic::Ordering;
    let mock = crate::auth_worker_mock::mock_server(false).await;
    let plane = test_plane().await;
    seed_promoted_profile_and_fleet(&plane).await;
    let before = plane
        .control_plane
        .fleet_auth_context_get(FLEET)
        .await
        .unwrap()
        .unwrap();
    mock.repo_id.store(999, Ordering::SeqCst);
    let mut wiring = wiring_with(
        &plane.control_plane,
        crate::auth_worker_mock::endpoints(&mock.base),
    )
    .await;
    tick_and_drain(&mut wiring, 1_800_000_000_000).await;
    let after = plane
        .control_plane
        .fleet_auth_context_get(FLEET)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.desired_context_json, before.desired_context_json);
    assert!(after.observed_context_json.is_none());
    assert!(plane
        .control_plane
        .handoff_get(FLEET)
        .await
        .unwrap()
        .unwrap()
        .observed
        .is_none());
    assert_eq!(mock.scale_set_creates.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn dynamic_candidate_preserves_live_repository_identity() {
    use std::sync::atomic::Ordering;
    let mock = crate::auth_worker_mock::mock_server(false).await;
    let plane = test_plane().await;
    let fence = seed_promoted_profile_and_fleet(&plane).await;
    acknowledge_handoff(&plane, fence).await;
    let before = plane
        .control_plane
        .fleet_auth_context_get(FLEET)
        .await
        .unwrap()
        .unwrap();
    crate::auth_worker_v2::tests::seed_candidate(
        &plane.control_plane,
        2,
        r#"{"selectors":[{"kind":"account_repositories","account_kind":"user","owner":"5aaee9"}]}"#,
    )
    .await;
    mock.repo_id.store(999, Ordering::SeqCst);
    let store: Arc<dyn ControlPlaneStore> = plane.control_plane.clone();
    let row = store.auth_revision_get(KEY, 2).await.unwrap().unwrap();
    let verdict = crate::auth_worker_v2::validate_v2(
        &store,
        &(Arc::new(crate::auth_worker_mock::Now) as Arc<dyn shaula_core::ports::Clock>),
        KEY,
        &row,
        &crate::auth_worker_mock::endpoints(&mock.base),
    )
    .await
    .unwrap();
    assert_eq!(verdict, crate::auth_worker_v2::Verdict::Rejected);
    assert_eq!(
        store
            .auth_profile_get(KEY)
            .await
            .unwrap()
            .unwrap()
            .active_revision,
        Some(1)
    );
    assert!(store.auth_bindings_get(KEY, 2).await.unwrap().is_empty());
    let after = store.fleet_auth_context_get(FLEET).await.unwrap().unwrap();
    assert_eq!(after.desired_context_json, before.desired_context_json);
    assert_eq!(after.observed_context_json, before.observed_context_json);
}
