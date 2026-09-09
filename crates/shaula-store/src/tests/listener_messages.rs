//! Real SQLite commit/ACK/acquire regressions for the production listener port.
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use shaula_core::ports::{
    JobMessage, JobStartedMessage, PollMessage, SessionHandle, StatisticsSnapshot,
};
use shaula_core::registry::{FleetRuntimeGuard, SessionEffectContext, SessionInstall};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

pub(crate) async fn ready() -> TestResult<(crate::Store, SessionEffectContext)> {
    let (store, captured, auth_context) = super::auth_execution::ready().await;
    store
        .handoff_acknowledge(
            "fleet",
            &auth_context.profile_key,
            auth_context.revision,
            Some(&serde_json::to_string(&auth_context)?),
            &captured,
        )
        .await?;
    store
        .scale_set_upsert(shaula_core::registry::ScaleSetRow {
            fleet_key: "fleet".into(),
            scale_set_id: Some(1),
            name: "test".into(),
            runner_group: "default".into(),
            fingerprint: "fixture".into(),
            state: "Adopted".into(),
            attempt_id: None,
            now: 5,
        })
        .await?;
    let guard = FleetRuntimeGuard {
        incarnation: "inc-fleet".into(),
        desired_revision: 1,
        mutation_fence: 1,
    };
    let install = SessionInstall {
        guard: guard.clone(),
        expected_epoch: None,
        auth_context: auth_context.clone(),
        scale_set_id: 1,
        handle: SessionHandle {
            session_id: "session-1".into(),
            message_queue_url: "https://queue.example.test/".into(),
            message_queue_access_token: "test-queue-token".into(),
            initial_statistics: StatisticsSnapshot::default(),
        },
    };
    let epoch = store
        .session_install("fleet", &install, 10)
        .await?
        .ok_or("fixture session rejected")?;
    Ok((
        store,
        SessionEffectContext {
            guard,
            auth_context,
            epoch,
        },
    ))
}

pub(super) fn message(id: i64, demand: i64) -> PollMessage {
    PollMessage {
        message_id: id,
        statistics: StatisticsSnapshot {
            total_assigned_jobs: demand,
            ..Default::default()
        },
        job_available: vec![JobMessage {
            runner_request_id: 101,
            job_id: "job-1".into(),
            metadata: Default::default(),
        }],
        job_assigned: vec![],
        job_started: vec![JobStartedMessage {
            runner_request_id: 99,
            job_id: "job-old".into(),
            runner_id: 2,
            runner_name: "runner-2".into(),
            metadata: Default::default(),
        }],
        job_completed: vec![],
    }
}

pub(super) async fn number(store: &crate::Store, sql: &str) -> TestResult<i64> {
    let row = store
        .connection()
        .query_one(Statement::from_string(DbBackend::Sqlite, sql))
        .await?
        .ok_or("missing count")?;
    Ok(row.try_get("", "value")?)
}

#[tokio::test]
async fn ingest_commits_all_facts_before_ack_and_recovers_pending_work() -> TestResult {
    let (store, context) = ready().await?;
    let ingested = store
        .listener_ingest("fleet", &context, &message(1, 3), 20)
        .await?
        .ok_or("stale")?;
    assert!(!ingested.acked);
    assert_eq!(ingested.pending_request_ids, [101]);
    assert_eq!(
        number(
            &store,
            "SELECT total_assigned_jobs AS value FROM fleet_demand"
        )
        .await?,
        3
    );
    assert_eq!(
        number(
            &store,
            "SELECT COUNT(*) AS value FROM listener_job_observations"
        )
        .await?,
        1
    );
    assert_eq!(
        number(
            &store,
            "SELECT last_message_id AS value FROM fleet_sessions"
        )
        .await?,
        0
    );
    assert_eq!(
        store.listener_pending("fleet", &context).await?,
        vec![ingested]
    );
    assert!(store
        .listener_acquire_start("fleet", &context, 1, 21)
        .await
        .is_err());
    assert!(store.listener_acknowledge("fleet", &context, 1, 22).await?);
    assert_eq!(
        number(
            &store,
            "SELECT last_message_id AS value FROM fleet_sessions"
        )
        .await?,
        1
    );
    assert!(store.listener_pending("fleet", &context).await?[0].acked);
    assert_eq!(
        store
            .listener_acquire_start("fleet", &context, 1, 23)
            .await?,
        Some(vec![101])
    );
    assert_eq!(
        store
            .listener_acquire_start("fleet", &context, 1, 24)
            .await?,
        Some(vec![])
    );
    assert!(
        store
            .listener_acquire_complete("fleet", &context, 1, Some(&[101]), 25)
            .await?
    );
    assert!(store.listener_pending("fleet", &context).await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn failed_message_transaction_leaves_no_checkpoint_demand_observation_or_acquisition(
) -> TestResult {
    let (store, context) = ready().await?;
    store.connection().execute_unprepared("CREATE TRIGGER reject_listener_intent BEFORE INSERT ON listener_acquisitions BEGIN SELECT RAISE(ABORT,'injected failure'); END").await?;
    assert!(store
        .listener_ingest("fleet", &context, &message(1, 9), 20)
        .await
        .is_err());
    for table in [
        "listener_messages",
        "listener_acquisitions",
        "listener_job_observations",
    ] {
        assert_eq!(
            number(&store, &format!("SELECT COUNT(*) AS value FROM {table}")).await?,
            0
        );
    }
    assert_eq!(
        number(
            &store,
            "SELECT total_assigned_jobs AS value FROM fleet_demand"
        )
        .await?,
        0
    );
    assert_eq!(
        number(
            &store,
            "SELECT last_message_id AS value FROM fleet_sessions"
        )
        .await?,
        0
    );
    assert!(!store.listener_acknowledge("fleet", &context, 1, 21).await?);
    store
        .connection()
        .execute_unprepared("DROP TRIGGER reject_listener_intent")
        .await?;
    assert!(store
        .listener_ingest("fleet", &context, &message(1, 9), 22)
        .await?
        .is_some());
    Ok(())
}

#[tokio::test]
async fn redelivery_does_not_rewind_statistics_or_duplicate_acquisition() -> TestResult {
    let (store, context) = ready().await?;
    let first = message(1, 3);
    store.listener_ingest("fleet", &context, &first, 20).await?;
    store
        .listener_ingest("fleet", &context, &message(2, 7), 21)
        .await?;
    store.listener_ingest("fleet", &context, &first, 22).await?;
    assert_eq!(
        number(
            &store,
            "SELECT total_assigned_jobs AS value FROM fleet_demand"
        )
        .await?,
        7
    );
    assert_eq!(
        number(
            &store,
            "SELECT COUNT(*) AS value FROM listener_acquisitions"
        )
        .await?,
        1
    );
    assert_eq!(
        number(
            &store,
            "SELECT COUNT(*) AS value FROM listener_job_observations"
        )
        .await?,
        2
    );
    assert!(store
        .listener_ingest("fleet", &context, &message(1, 20), 23)
        .await
        .is_err());
    assert_eq!(
        number(
            &store,
            "SELECT total_assigned_jobs AS value FROM fleet_demand"
        )
        .await?,
        7
    );
    Ok(())
}

#[tokio::test]
async fn stale_epoch_head_and_authority_cannot_ingest_ack_or_start_acquisition() -> TestResult {
    let (store, context) = ready().await?;
    store
        .listener_ingest("fleet", &context, &message(1, 3), 20)
        .await?;
    let mut candidates = vec![];
    let mut stale = context.clone();
    stale.epoch += 1;
    candidates.push(stale);
    let mut stale = context.clone();
    stale.guard.mutation_fence += 1;
    candidates.push(stale);
    let mut stale = context.clone();
    stale.guard.desired_revision += 1;
    candidates.push(stale);
    let mut stale = context.clone();
    stale.auth_context.installation_id += 1;
    candidates.push(stale);
    for stale in candidates {
        assert!(store
            .listener_ingest("fleet", &stale, &message(2, 10), 21)
            .await?
            .is_none());
        assert!(!store.listener_acknowledge("fleet", &stale, 1, 22).await?);
        assert!(store
            .listener_acquire_start("fleet", &stale, 1, 23)
            .await?
            .is_none());
    }
    store
        .connection()
        .execute_unprepared("UPDATE fleets SET deletion_marker=1 WHERE key='fleet'")
        .await?;
    assert!(store
        .listener_ingest("fleet", &context, &message(2, 10), 24)
        .await?
        .is_none());
    assert!(!store.listener_acknowledge("fleet", &context, 1, 25).await?);
    assert_eq!(
        number(
            &store,
            "SELECT total_assigned_jobs AS value FROM fleet_demand"
        )
        .await?,
        3
    );
    Ok(())
}

#[tokio::test]
async fn stale_acquisition_completion_retains_result_without_waking_or_writing_demand() -> TestResult
{
    let (store, context) = ready().await?;
    store
        .listener_ingest("fleet", &context, &message(1, 3), 20)
        .await?;
    store.listener_acknowledge("fleet", &context, 1, 21).await?;
    store
        .listener_acquire_start("fleet", &context, 1, 22)
        .await?;
    let wakes = number(&store, "SELECT COUNT(*) AS value FROM outbox").await?;
    store
        .connection()
        .execute_unprepared("UPDATE fleets SET mutation_fence=2 WHERE key='fleet'")
        .await?;
    assert!(
        !store
            .listener_acquire_complete("fleet", &context, 1, Some(&[101]), 23)
            .await?
    );
    assert_eq!(
        number(
            &store,
            "SELECT COUNT(*) AS value FROM listener_acquisitions WHERE state='Acquired'"
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
        3
    );
    assert_eq!(
        number(&store, "SELECT COUNT(*) AS value FROM outbox").await?,
        wakes
    );
    Ok(())
}

#[tokio::test]
async fn uncertain_acquisition_is_not_retried_on_redelivery() -> TestResult {
    let (store, context) = ready().await?;
    let first = message(1, 3);
    store.listener_ingest("fleet", &context, &first, 20).await?;
    store.listener_acknowledge("fleet", &context, 1, 21).await?;
    store
        .listener_acquire_start("fleet", &context, 1, 22)
        .await?;
    assert!(store
        .listener_acquire_complete("fleet", &context, 1, Some(&[999]), 23)
        .await
        .is_err());
    store
        .listener_acquire_complete("fleet", &context, 1, None, 24)
        .await?;
    assert!(store
        .listener_ingest("fleet", &context, &first, 25)
        .await?
        .ok_or("stale")?
        .pending_request_ids
        .is_empty());
    assert_eq!(
        store
            .listener_acquire_start("fleet", &context, 1, 26)
            .await?,
        Some(vec![])
    );
    assert_eq!(
        number(
            &store,
            "SELECT COUNT(*) AS value FROM listener_acquisitions WHERE state='Uncertain'"
        )
        .await?,
        1
    );
    Ok(())
}
