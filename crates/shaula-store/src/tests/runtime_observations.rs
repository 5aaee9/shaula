//! Fleet and Change observations advance together and reject stale work.

use sea_orm::ConnectionTrait;
use shaula_core::error::ReasonCode;
use shaula_core::registry::{FleetObservation, FleetObservationPhase};

use super::runtime_sessions::established;
use super::session_support::request;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

struct FixedClock;
impl shaula_core::ports::Clock for FixedClock {
    fn now_unix_ms(&self) -> i64 {
        20
    }
}

async fn setup() -> TestResult<(crate::Store, FleetObservation)> {
    let (store, context) = established().await?;
    let tx = store.begin().await?;
    store
        .change_insert(&tx, "change", "fleet", 1, "Create", 1)
        .await?;
    tx.commit().await?;
    let guard = request(&store, "fleet", "next", 42, &context).await?.guard;
    Ok((
        store,
        FleetObservation {
            guard,
            session_epoch: Some(1),
            phase: FleetObservationPhase::Reconciling,
            reason: None,
        },
    ))
}

#[tokio::test]
async fn fleet_and_change_expose_blocking_reason_and_clear_it_on_recovery() -> TestResult {
    let (store, mut observed) = setup().await?;
    assert!(store.fleet_set_observed("fleet", &observed, 11).await?);
    let fleet = store.fleet_get("fleet").await?.ok_or("missing fleet")?;
    assert_eq!(fleet.phase, "Reconciling");
    assert_eq!(fleet.observed_revision, 1);
    assert_eq!(
        store
            .change_get("change")
            .await?
            .ok_or("missing change")?
            .state,
        "Running"
    );

    observed.phase = FleetObservationPhase::Degraded;
    observed.reason = Some(ReasonCode::PermissionDenied);
    assert!(store.fleet_set_observed("fleet", &observed, 12).await?);
    assert_eq!(
        store
            .fleet_get("fleet")
            .await?
            .ok_or("missing fleet")?
            .last_condition_reason
            .as_deref(),
        Some("PermissionDenied")
    );
    let change = store.change_get("change").await?.ok_or("missing change")?;
    assert_eq!(change.state, "Blocked");
    assert_eq!(change.reason.as_deref(), Some("PermissionDenied"));

    observed.phase = FleetObservationPhase::Ready;
    observed.reason = None;
    assert!(store.fleet_set_observed("fleet", &observed, 13).await?);
    let fleet = store.fleet_get("fleet").await?.ok_or("missing fleet")?;
    assert_eq!(fleet.phase, "Ready");
    assert!(fleet.last_condition_reason.is_none());
    let change = store.change_get("change").await?.ok_or("missing change")?;
    assert_eq!(change.state, "Succeeded");
    assert!(change.reason.is_none());
    // A later runtime error is visible on Fleet, but cannot reopen a terminal Change.
    observed.phase = FleetObservationPhase::Degraded;
    observed.reason = Some(ReasonCode::RateLimited);
    assert!(store.fleet_set_observed("fleet", &observed, 14).await?);
    assert_eq!(
        store
            .change_get("change")
            .await?
            .ok_or("missing change")?
            .state,
        "Succeeded"
    );
    Ok(())
}
#[tokio::test]
async fn late_observation_never_overwrites_replacement_deletion_or_new_session() -> TestResult {
    for mutation in [
        "UPDATE fleets SET desired_revision=2 WHERE key='fleet'",
        "UPDATE fleets SET incarnation='new' WHERE key='fleet'",
        "UPDATE fleets SET mutation_fence=2 WHERE key='fleet'",
        "UPDATE fleets SET deletion_marker=1,phase='Decommissioning' WHERE key='fleet'",
        "UPDATE fleets SET tombstone=1,phase='Decommissioned' WHERE key='fleet'",
        "UPDATE fleet_sessions SET epoch=2 WHERE fleet_key='fleet'",
    ] {
        let (store, observed) = setup().await?;
        store.connection().execute_unprepared(mutation).await?;
        let previous = store.fleet_get("fleet").await?.ok_or("missing fleet")?;
        assert!(
            !store.fleet_set_observed("fleet", &observed, 12).await?,
            "{mutation}"
        );
        assert_eq!(
            store.fleet_get("fleet").await?.ok_or("missing fleet")?,
            previous,
            "{mutation}"
        );
        assert_eq!(
            store
                .change_get("change")
                .await?
                .ok_or("missing change")?
                .state,
            "Pending"
        );
    }
    Ok(())
}
#[tokio::test]
async fn ready_requires_a_current_listener_and_exact_observed_authentication() -> TestResult {
    for mutation in [
        "UPDATE fleet_sessions SET queue_token=NULL WHERE fleet_key='fleet'",
        "UPDATE fleet_auth_handoffs SET desired_revision=2 WHERE fleet_key='fleet'",
        "UPDATE fleet_auth_contexts SET state='Blocked' WHERE fleet_key='fleet'",
        "UPDATE scale_set_state SET state='AccessBlocked' WHERE fleet_key='fleet'",
    ] {
        let (store, mut observed) = setup().await?;
        observed.phase = FleetObservationPhase::Ready;
        store.connection().execute_unprepared(mutation).await?;
        assert!(
            !store.fleet_set_observed("fleet", &observed, 12).await?,
            "{mutation}"
        );
        assert_eq!(
            store
                .fleet_get("fleet")
                .await?
                .ok_or("missing fleet")?
                .phase,
            "Pending"
        );
        assert_eq!(
            store
                .change_get("change")
                .await?
                .ok_or("missing change")?
                .state,
            "Pending"
        );
    }
    Ok(())
}

#[tokio::test]
async fn status_read_exposes_persisted_failure_in_last_error_and_converged_condition() -> TestResult
{
    use shaula_core::registry::{Actor, FleetRegistryPort, Scope};
    use std::sync::Arc;

    let (store, mut observation) = setup().await?;
    let service = shaula_daemon::service::ControlPlane::new(
        Arc::new(crate::registry_impl::SqliteControlPlane::new(
            store.clone(),
            "unused".into(),
        )),
        Arc::new(FixedClock),
        Vec::new(),
        10,
        "unused".into(),
    );
    let actor = Actor {
        name: "operator".into(),
        scopes: vec![Scope::FleetRead],
    };
    observation.phase = FleetObservationPhase::Degraded;
    observation.reason = Some(ReasonCode::OwnershipProofFailed);
    assert!(store.fleet_set_observed("fleet", &observation, 20).await?);
    let status = service
        .fleet_status_get(&actor, "fleet")
        .await?
        .map_err(|e| format!("status read: {e:?}"))?;
    assert_eq!(status.phase, "Degraded");
    assert_eq!(status.last_error.as_deref(), Some("OwnershipProofFailed"));
    let converged = status
        .conditions
        .iter()
        .find(|c| c.condition_type == "Converged")
        .ok_or("missing convergence condition")?;
    assert!(!converged.status);
    assert_eq!(converged.reason.as_deref(), Some("OwnershipProofFailed"));

    observation.phase = FleetObservationPhase::Ready;
    observation.reason = None;
    assert!(store.fleet_set_observed("fleet", &observation, 21).await?);
    let recovered = service
        .fleet_status_get(&actor, "fleet")
        .await?
        .map_err(|e| format!("status read: {e:?}"))?;
    assert_eq!(recovered.phase, "Ready");
    assert_eq!(recovered.observed_revision, recovered.desired_revision);
    assert!(recovered.last_error.is_none());
    // Listener recovery clears the failure but does not prove capacity convergence.
    assert!(recovered
        .conditions
        .iter()
        .any(|c| c.condition_type == "Converged" && !c.status && c.reason.is_none()));
    Ok(())
}
