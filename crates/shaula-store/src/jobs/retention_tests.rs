use sea_orm::ConnectionTrait;
use shaula_core::jobs::{GenerationsQuery, JobsQuery, JobsReadPort};

use super::{generation, generation_with_runner, message, TestResult};

#[tokio::test]
async fn pruning_preserves_live_generation_and_retained_log_evidence() -> TestResult {
    let (store, context) = crate::tests::listener_messages::ready().await?;
    generation(&store, "generation", "runner-42").await?;
    let mut event = message(1, "job");
    event.job_started.push(started("job", 42, "runner-42"));
    store.listener_ingest("fleet", &context, &event, 20).await?;
    store.prune_job_history(100).await?;
    assert_eq!(store.list_jobs(JobsQuery::default()).await?.items.len(), 1);
    store.connection().execute_unprepared("UPDATE runner_generations SET state='Destroyed',updated_at=30;
        INSERT INTO operation_log_invocations(id,generation_id,fleet_key,operation,ordinal,record_json,sealed_at)
        VALUES('log','generation','fleet','apply',1,'{}',30)").await?;
    store.prune_job_history(100).await?;
    assert_eq!(store.list_jobs(JobsQuery::default()).await?.items.len(), 1);
    store
        .connection()
        .execute_unprepared("DELETE FROM operation_log_invocations")
        .await?;
    store.prune_job_history(100).await?;
    assert!(store
        .list_jobs(JobsQuery::default())
        .await?
        .items
        .is_empty());
    // Resource evidence remains owned by the lifecycle ledger.
    assert_eq!(
        store
            .list_generations(GenerationsQuery::default())
            .await?
            .items
            .len(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn unrelated_active_runner_and_capture_do_not_keep_old_completed_job() -> TestResult {
    let (store, context) = crate::tests::listener_messages::ready().await?;
    generation_with_runner(&store, "finished", "runner-41", 41).await?;
    let mut event = message(1, "old-job");
    event.job_started.push(started("old-job", 41, "runner-41"));
    event
        .job_completed
        .push(shaula_core::ports::JobCompletedMessage {
            runner_request_id: 71,
            job_id: "old-job".into(),
            runner_id: 41,
            runner_name: "runner-41".into(),
            metadata: Default::default(),
            result: Some("succeeded".into()),
        });
    store.listener_ingest("fleet", &context, &event, 20).await?;
    store
        .connection()
        .execute_unprepared(
            "UPDATE runner_generations SET state='Destroyed',updated_at=30 WHERE id='finished'",
        )
        .await?;
    generation(&store, "warm", "runner-42").await?;
    store.connection().execute_unprepared("INSERT INTO operation_log_invocations(id,generation_id,fleet_key,operation,ordinal,record_json)
        VALUES('active-log','warm','fleet','Create',1,'{}')").await?;
    store.prune_job_history(100).await?;
    assert!(store
        .list_jobs(JobsQuery::default())
        .await?
        .items
        .is_empty());
    assert_eq!(
        store
            .list_generations(GenerationsQuery::default())
            .await?
            .items
            .len(),
        2
    );
    Ok(())
}

#[tokio::test]
async fn unresolved_facts_protect_only_their_numeric_runner_candidate() -> TestResult {
    let (store, context) = crate::tests::listener_messages::ready().await?;
    generation(&store, "warm", "runner-42").await?;
    let mut anonymous = message(1, "");
    anonymous.job_available[0].runner_request_id = 999;
    store
        .listener_ingest("fleet", &context, &anonymous, 20)
        .await?;
    let mut matched = message(2, "");
    matched.job_available.clear();
    matched.job_started.push(started("", 42, "runner-42"));
    store
        .listener_ingest("fleet", &context, &matched, 30)
        .await?;
    store.prune_job_history(100).await?;
    let remaining = super::super::rows(
        store.connection(),
        "SELECT runner_id FROM workflow_job_observations WHERE job_record_id IS NULL",
        vec![],
    )
    .await?;
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].try_get::<i64>("", "runner_id")?, 42);
    Ok(())
}

fn started(job: &str, runner_id: i64, runner_name: &str) -> shaula_core::ports::JobStartedMessage {
    shaula_core::ports::JobStartedMessage {
        runner_request_id: 71,
        job_id: job.into(),
        runner_id,
        runner_name: runner_name.into(),
        metadata: Default::default(),
    }
}

#[tokio::test]
async fn legacy_numeric_runner_without_scope_stays_unassigned() -> TestResult {
    let (store, context) = crate::tests::listener_messages::ready().await?;
    generation(&store, "legacy", "runner-42").await?;
    store
        .connection()
        .execute_unprepared("DELETE FROM workflow_generation_identity")
        .await?;
    let mut event = message(1, "job");
    event
        .job_started
        .push(shaula_core::ports::JobStartedMessage {
            runner_request_id: 71,
            job_id: "job".into(),
            runner_id: 42,
            runner_name: "runner-42".into(),
            metadata: Default::default(),
        });
    store.listener_ingest("fleet", &context, &event, 20).await?;
    let jobs = store.list_jobs(JobsQuery::default()).await?;
    assert_eq!(
        jobs.items[0].observed_status,
        shaula_core::jobs::ObservedStatus::Unknown
    );
    assert!(store
        .get_job(&jobs.items[0].id)
        .await?
        .ok_or("missing")?
        .generations
        .is_empty());
    let generations = store
        .list_generations(GenerationsQuery {
            association: Some("unassigned".into()),
            ..Default::default()
        })
        .await?;
    assert_eq!(generations.items.len(), 1);
    assert_eq!(generations.items[0].fleet_incarnation, None);
    Ok(())
}

#[tokio::test]
async fn session_reconnect_keeps_the_same_job_identity() -> TestResult {
    let (store, mut context) = crate::tests::listener_messages::ready().await?;
    store
        .listener_ingest("fleet", &context, &message(1, "job"), 20)
        .await?;
    let original = store.list_jobs(JobsQuery::default()).await?.items.remove(0);
    let install = shaula_core::registry::SessionInstall {
        guard: context.guard.clone(),
        auth_context: context.auth_context.clone(),
        expected_epoch: Some(context.epoch),
        scale_set_id: 1,
        handle: shaula_core::ports::SessionHandle {
            session_id: "replacement".into(),
            message_queue_url: "https://queue.example.test".into(),
            message_queue_access_token: "token".into(),
            initial_statistics: Default::default(),
        },
    };
    context.epoch = store
        .session_install("fleet", &install, 30)
        .await?
        .ok_or("session rejected")?;
    store
        .listener_ingest("fleet", &context, &message(1, "job"), 40)
        .await?;
    let current = store.list_jobs(JobsQuery::default()).await?;
    assert_eq!(current.items.len(), 1);
    assert_eq!(current.items[0].id, original.id);
    assert_eq!(
        store
            .get_job(&original.id)
            .await?
            .ok_or("missing")?
            .observations
            .len(),
        2
    );
    Ok(())
}
