//! Epoch replacement and durable acquisition-outcome recovery boundaries.
use sea_orm::ConnectionTrait;
use shaula_core::registry::SessionInstall;

use super::listener_messages::{message, number, ready};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[tokio::test]
async fn acknowledgement_cannot_skip_an_earlier_unacknowledged_message() -> TestResult {
    let (store, context) = ready().await?;
    store
        .listener_ingest("fleet", &context, &message(1, 1), 20)
        .await?;
    store
        .listener_ingest("fleet", &context, &message(2, 2), 21)
        .await?;
    assert!(store
        .listener_acknowledge("fleet", &context, 2, 22)
        .await
        .is_err());
    assert_eq!(
        number(
            &store,
            "SELECT last_message_id AS value FROM fleet_sessions"
        )
        .await?,
        0
    );
    assert!(store.listener_acknowledge("fleet", &context, 1, 23).await?);
    assert!(store.listener_acknowledge("fleet", &context, 2, 24).await?);
    assert_eq!(
        number(
            &store,
            "SELECT last_message_id AS value FROM fleet_sessions"
        )
        .await?,
        2
    );
    Ok(())
}

#[tokio::test]
async fn replacement_classifies_old_work_and_same_message_id_is_a_new_epoch() -> TestResult {
    let (store, old) = ready().await?;
    store
        .listener_ingest("fleet", &old, &message(1, 3), 20)
        .await?;
    store.listener_acknowledge("fleet", &old, 1, 21).await?;
    store.listener_acquire_start("fleet", &old, 1, 22).await?;
    let mut second = message(2, 5);
    second.job_available[0].runner_request_id = 102;
    store.listener_ingest("fleet", &old, &second, 23).await?;
    let mut handle = store
        .session_handle("fleet")
        .await?
        .ok_or("session missing")?
        .handle;
    handle.session_id = "replacement".into();
    handle.initial_statistics.total_assigned_jobs = 8;
    let install = SessionInstall {
        guard: old.guard.clone(),
        expected_epoch: Some(old.epoch),
        auth_context: old.auth_context.clone(),
        scale_set_id: 1,
        handle,
    };
    let epoch = store
        .session_install("fleet", &install, 24)
        .await?
        .ok_or("replacement stale")?;
    assert!(epoch > old.epoch);
    assert_eq!(
        number(
            &store,
            "SELECT COUNT(*) AS value FROM listener_acquisitions WHERE state='ReconciledUncertain'"
        )
        .await?,
        1
    );
    assert_eq!(
        number(
            &store,
            "SELECT COUNT(*) AS value FROM listener_acquisitions WHERE state='Cancelled'"
        )
        .await?,
        1
    );
    assert_eq!(
        number(
            &store,
            "SELECT total_assigned_jobs AS value FROM fleet_demand"
        )
        .await?,
        8
    );
    assert!(store
        .listener_ingest("fleet", &old, &message(3, 99), 25)
        .await?
        .is_none());
    let mut new = old.clone();
    new.epoch = epoch;
    let new_message = store
        .listener_ingest("fleet", &new, &message(1, 7), 26)
        .await?
        .ok_or("new epoch stale")?;
    assert_eq!(new_message.pending_request_ids, [101]);
    assert_eq!(
        number(
            &store,
            "SELECT COUNT(*) AS value FROM listener_job_observations WHERE message_id=1"
        )
        .await?,
        2
    );
    assert_eq!(
        number(
            &store,
            "SELECT last_message_id AS value FROM fleet_sessions"
        )
        .await?,
        0
    );
    Ok(())
}

#[tokio::test]
async fn corrupt_current_auth_does_not_rollback_an_already_started_acquisition_result() -> TestResult
{
    let (store, context) = ready().await?;
    store
        .listener_ingest("fleet", &context, &message(1, 3), 20)
        .await?;
    store.listener_acknowledge("fleet", &context, 1, 21).await?;
    store
        .listener_acquire_start("fleet", &context, 1, 22)
        .await?;
    store
        .connection()
        .execute_unprepared("UPDATE github_auth_profile_revisions SET schema_version=1")
        .await?;
    assert!(store
        .listener_acquire_complete("fleet", &context, 1, Some(&[101]), 23)
        .await
        .is_err());
    assert_eq!(
        number(
            &store,
            "SELECT COUNT(*) AS value FROM listener_acquisitions WHERE state='Acquired'"
        )
        .await?,
        1
    );
    Ok(())
}
