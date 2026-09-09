//! GitHub can assign a job before it has a runner request identity.
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use shaula_core::jobs::JobMetadata;

use super::*;

fn assigned(id: i64, jobs: &[&str]) -> PollMessage {
    let mut message = message(id, "");
    message.job_available.clear();
    message.statistics.total_assigned_jobs = 1;
    message.job_assigned = jobs
        .iter()
        .map(|job| JobMessage {
            runner_request_id: 0,
            job_id: (*job).into(),
            metadata: JobMetadata {
                scale_set_assign_time: chrono::DateTime::from_timestamp(20, 0),
                ..Default::default()
            },
        })
        .collect();
    message
}

async fn count(store: &crate::Store, table: &str) -> Result<i64, Box<dyn std::error::Error>> {
    let row = store
        .connection()
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            format!("SELECT COUNT(*) AS count FROM {table}"),
        ))
        .await?
        .ok_or("missing count")?;
    Ok(row.try_get("", "count")?)
}

#[tokio::test]
async fn zero_request_assignments_commit_before_ack_without_merging_or_acquiring() -> TestResult {
    let (store, context) = crate::tests::listener_messages::ready().await?;
    let message = assigned(1, &["job-a", "job-b", "", ""]);
    let ingested = store
        .listener_ingest("fleet", &context, &message, 20)
        .await?
        .ok_or("stale session")?;
    assert!(!ingested.acked);
    assert!(ingested.pending_request_ids.is_empty());
    assert_eq!(count(&store, "workflow_job_observations").await?, 4);
    assert_eq!(count(&store, "listener_acquisitions").await?, 0);
    assert_eq!(
        store
            .demand_get("fleet")
            .await?
            .map(|demand| demand.total_assigned_jobs),
        Some(1)
    );
    assert_eq!(
        store
            .session_get("fleet")
            .await?
            .ok_or("session")?
            .last_message_id,
        0
    );
    let jobs = store.list_jobs(JobsQuery::default()).await?;
    assert_eq!(jobs.items.len(), 2);
    for job in jobs.items {
        let detail = store.get_job(&job.id).await?.ok_or("job missing")?;
        assert_eq!(detail.observations.len(), 1);
        assert_eq!(detail.observations[0].runner_request_id, 0);
        assert_eq!(detail.job.observed_status, ObservedStatus::Assigned);
        assert_eq!(detail.job.association_status, AssociationStatus::Unverified);
    }
    assert!(store.listener_acknowledge("fleet", &context, 1, 21).await?);
    assert_eq!(
        store
            .session_get("fleet")
            .await?
            .ok_or("session")?
            .last_message_id,
        1
    );
    assert_eq!(
        store
            .listener_acquire_start("fleet", &context, 1, 22)
            .await?,
        Some(vec![])
    );
    assert!(
        store
            .listener_ingest("fleet", &context, &message, 23)
            .await?
            .ok_or("stale")?
            .acked
    );
    let mut changed = message.clone();
    changed.job_assigned[0].runner_request_id = 1;
    assert!(store
        .listener_ingest("fleet", &context, &changed, 24)
        .await
        .is_err());
    assert_eq!(count(&store, "workflow_job_observations").await?, 4);
    Ok(())
}

#[tokio::test]
async fn unknown_assignment_request_does_not_hide_later_proven_execution() -> TestResult {
    let (store, context) = crate::tests::listener_messages::ready().await?;
    generation(&store, "generation", "runner-42").await?;
    store
        .listener_ingest("fleet", &context, &assigned(1, &["", "job-a"]), 20)
        .await?;
    let mut started = assigned(2, &[]);
    started.job_started.push(JobStartedMessage {
        runner_request_id: 71,
        job_id: "job-a".into(),
        runner_id: 42,
        runner_name: "runner-42".into(),
        metadata: JobMetadata {
            scale_set_assign_time: chrono::DateTime::from_timestamp(20, 0),
            ..Default::default()
        },
    });
    store
        .listener_ingest("fleet", &context, &started, 30)
        .await?;
    store
        .listener_ingest("fleet", &context, &assigned(3, &["job-b"]), 40)
        .await?;
    let jobs = store.list_jobs(JobsQuery::default()).await?;
    let job = jobs
        .items
        .iter()
        .find(|job| job.protocol_job_id == "job-a")
        .ok_or("job-a missing")?;
    let detail = store.get_job(&job.id).await?.ok_or("job missing")?;
    assert_eq!(detail.job.observed_status, ObservedStatus::Running);
    assert_eq!(detail.job.association_status, AssociationStatus::Verified);
    assert_eq!(
        detail.observations.len(),
        2,
        "unresolved zero request cannot join this job"
    );
    assert_eq!(detail.generations[0].id, "generation");
    assert_eq!(count(&store, "workflow_job_observations").await?, 4);
    Ok(())
}

#[tokio::test]
async fn available_zero_and_negative_observation_requests_remain_invalid() -> TestResult {
    let (store, context) = crate::tests::listener_messages::ready().await?;
    let mut invalid = vec![];
    let mut available = message(1, "job-a");
    available.job_available[0].runner_request_id = 0;
    invalid.push(available);
    let mut negative = assigned(1, &["job-a"]);
    negative.job_assigned[0].runner_request_id = -1;
    invalid.push(negative);
    let mut started = assigned(1, &[]);
    started.job_started.push(JobStartedMessage {
        runner_request_id: -1,
        job_id: "job-a".into(),
        runner_id: 42,
        runner_name: "runner-42".into(),
        metadata: Default::default(),
    });
    invalid.push(started);
    let mut completed = assigned(1, &[]);
    completed.job_completed.push(JobCompletedMessage {
        runner_request_id: -1,
        job_id: "job-a".into(),
        runner_id: 42,
        runner_name: "runner-42".into(),
        metadata: Default::default(),
        result: None,
    });
    invalid.push(completed);
    for message in invalid {
        assert!(store
            .listener_ingest("fleet", &context, &message, 20)
            .await
            .is_err());
        assert!(!store.listener_acknowledge("fleet", &context, 1, 21).await?);
        assert_eq!(count(&store, "workflow_job_observations").await?, 0);
        assert_eq!(count(&store, "listener_acquisitions").await?, 0);
        assert_eq!(
            store
                .demand_get("fleet")
                .await?
                .map(|demand| demand.total_assigned_jobs),
            Some(0)
        );
    }
    Ok(())
}

#[tokio::test]
async fn zero_request_execution_facts_keep_exact_runner_identity_and_completion() -> TestResult {
    let (store, context) = crate::tests::listener_messages::ready().await?;
    let mut message = assigned(1, &["job-a", "job-b"]);
    let metadata = message.job_assigned[0].metadata.clone();
    for (job, runner) in [("job-a", 42), ("job-b", 43)] {
        generation_with_runner(
            &store,
            &format!("generation-{runner}"),
            &format!("runner-{runner}"),
            runner,
        )
        .await?;
        message.job_started.push(JobStartedMessage {
            runner_request_id: 0,
            job_id: job.into(),
            runner_id: runner,
            runner_name: format!("runner-{runner}"),
            metadata: metadata.clone(),
        });
        message.job_completed.push(JobCompletedMessage {
            runner_request_id: 0,
            job_id: job.into(),
            runner_id: runner,
            runner_name: format!("runner-{runner}"),
            metadata: metadata.clone(),
            result: Some("succeeded".into()),
        });
    }
    store
        .listener_ingest("fleet", &context, &message, 20)
        .await?;
    let jobs = store.list_jobs(JobsQuery::default()).await?;
    assert_eq!(jobs.items.len(), 2);
    for (protocol_id, runner) in [("job-a", 42), ("job-b", 43)] {
        let job = jobs
            .items
            .iter()
            .find(|job| job.protocol_job_id == protocol_id)
            .ok_or("job missing")?;
        let detail = store.get_job(&job.id).await?.ok_or("job missing")?;
        assert_eq!(detail.observations.len(), 3);
        assert_eq!(detail.job.observed_status, ObservedStatus::Completed);
        assert_eq!(detail.job.association_status, AssociationStatus::Verified);
        assert_eq!(detail.generations.len(), 1);
        assert_eq!(detail.generations[0].id, format!("generation-{runner}"));
    }
    assert_eq!(count(&store, "workflow_job_observations").await?, 6);
    assert_eq!(count(&store, "listener_job_observations").await?, 0);
    assert!(store.listener_acknowledge("fleet", &context, 1, 21).await?);
    assert_eq!(
        store
            .listener_acquire_start("fleet", &context, 1, 22)
            .await?,
        Some(vec![])
    );
    Ok(())
}
