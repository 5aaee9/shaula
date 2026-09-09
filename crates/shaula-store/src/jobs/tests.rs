use shaula_core::jobs::{
    AssociationStatus, GenerationsQuery, JobsQuery, JobsReadPort, ObservedStatus,
};
use shaula_core::ports::{JobCompletedMessage, JobMessage, JobStartedMessage, PollMessage};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn message(id: i64, job: &str) -> PollMessage {
    PollMessage {
        message_id: id,
        statistics: Default::default(),
        job_available: vec![JobMessage {
            runner_request_id: 71,
            job_id: job.into(),
            metadata: Default::default(),
        }],
        job_assigned: vec![],
        job_started: vec![],
        job_completed: vec![],
    }
}

#[tokio::test]
async fn available_is_retained_and_redelivery_does_not_duplicate_jobs() -> TestResult {
    let (store, context) = crate::tests::listener_messages::ready().await?;
    let mut fact = message(1, "opaque-job");
    fact.job_available[0].metadata.owner_name = Some("example".into());
    fact.job_available[0].metadata.repository_name = Some("repo".into());
    fact.job_available[0].metadata.workflow_run_id = Some(17);
    store.listener_ingest("fleet", &context, &fact, 20).await?;
    store.listener_ingest("fleet", &context, &fact, 30).await?;
    let page = store.list_jobs(JobsQuery::default()).await?;
    assert_eq!(page.items.len(), 1);
    let job = &page.items[0];
    assert_eq!(job.observed_status, ObservedStatus::Queued);
    assert_eq!(
        job.github_run_url.as_deref(),
        Some("https://github.com/example/repo/actions/runs/17")
    );
    assert_eq!(job.actions_job_id, None);
    assert_eq!(job.github_conclusion, None);
    let detail = store.get_job(&job.id).await?.ok_or("missing job")?;
    assert_eq!(detail.observations.len(), 1);
    assert_eq!(detail.observations[0].observed_at, 20);
    Ok(())
}

#[tokio::test]
async fn unresolved_identity_is_promoted_only_from_consistent_scoped_request() -> TestResult {
    let (store, context) = crate::tests::listener_messages::ready().await?;
    store
        .listener_ingest("fleet", &context, &message(1, ""), 20)
        .await?;
    assert!(store
        .list_jobs(JobsQuery::default())
        .await?
        .items
        .is_empty());
    store
        .listener_ingest("fleet", &context, &message(2, "opaque"), 30)
        .await?;
    let page = store.list_jobs(JobsQuery::default()).await?;
    assert_eq!(
        store
            .get_job(&page.items[0].id)
            .await?
            .ok_or("missing")?
            .observations
            .len(),
        2
    );
    store
        .listener_ingest("fleet", &context, &message(3, "conflicting"), 40)
        .await?;
    assert_eq!(
        store
            .get_job(&page.items[0].id)
            .await?
            .ok_or("missing")?
            .observations
            .len(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn pagination_is_stable_and_cursor_cannot_change_filter() -> TestResult {
    let (store, context) = crate::tests::listener_messages::ready().await?;
    for id in 1..=3 {
        store
            .listener_ingest("fleet", &context, &message(id, &format!("job-{id}")), 20)
            .await?;
    }
    let query = JobsQuery {
        limit: Some(2),
        ..Default::default()
    };
    let first = store.list_jobs(query.clone()).await?;
    assert_eq!(first.items.len(), 2);
    let next = JobsQuery {
        cursor: first.next_cursor.clone(),
        ..query.clone()
    };
    let second = store.list_jobs(next.clone()).await?;
    assert_eq!(second.items.len(), 1);
    assert!(first.items.iter().all(|job| job.id != second.items[0].id));
    let mismatched = JobsQuery {
        fleet_key: Some("other".into()),
        ..next
    };
    assert!(matches!(
        store.list_jobs(mismatched).await,
        Err(shaula_core::jobs::JobsReadError::InvalidQuery(_))
    ));
    Ok(())
}

async fn generation(store: &crate::Store, id: &str, runner_name: &str) -> TestResult {
    generation_with_runner(store, id, runner_name, 42).await
}

async fn generation_with_runner(
    store: &crate::Store,
    id: &str,
    runner_name: &str,
    runner_id: i64,
) -> TestResult {
    store
        .generation_insert(shaula_core::registry::GenerationRecord {
            id: id.into(),
            fleet_key: "fleet".into(),
            runner_name: runner_name.into(),
            generation_name: id.into(),
            fleet_revision: 1,
            template_profile_key: "template".into(),
            template_revision: 1,
            template_artifact_digest: "digest".into(),
            attestation_id: "attestation".into(),
            inputs_digest: "inputs".into(),
            state: shaula_core::lifecycle::GenerationState::CreatePending,
            github_runner_id: None,
            workspace_path: "private-path".into(),
            created_at: 15,
            updated_at: 15,
        })
        .await?;
    store
        .generation_set_jit(id, "JITReady", Some(runner_id), 16)
        .await?;
    Ok(())
}

#[path = "retention_tests.rs"]
mod retention;

#[tokio::test]
async fn exact_numeric_runner_evidence_is_required_and_completion_needs_start() -> TestResult {
    let (store, context) = crate::tests::listener_messages::ready().await?;
    generation(&store, "generation", "runner-42").await?;
    let mut completed = message(1, "opaque");
    completed.job_completed.push(JobCompletedMessage {
        runner_request_id: 71,
        job_id: "opaque".into(),
        runner_id: 42,
        runner_name: "runner-42".into(),
        metadata: Default::default(),
        result: Some("succeeded".into()),
    });
    store
        .listener_ingest("fleet", &context, &completed, 20)
        .await?;
    let job = &store.list_jobs(JobsQuery::default()).await?.items[0];
    assert_eq!(job.observed_status, ObservedStatus::Unknown);
    let mut started = message(2, "opaque");
    started.job_started.push(JobStartedMessage {
        runner_request_id: 71,
        job_id: "opaque".into(),
        runner_id: 42,
        runner_name: "runner-42".into(),
        metadata: Default::default(),
    });
    store
        .listener_ingest("fleet", &context, &started, 30)
        .await?;
    let detail = store.get_job(&job.id).await?.ok_or("job missing")?;
    assert_eq!(detail.job.observed_status, ObservedStatus::Completed);
    assert_eq!(detail.generations[0].id, "generation");
    assert!(store
        .list_generations(GenerationsQuery {
            association: Some("unassigned".into()),
            ..Default::default()
        })
        .await?
        .items
        .is_empty());
    generation(&store, "ambiguous-generation", "other-runner").await?;
    let detail = store.get_job(&job.id).await?.ok_or("job missing")?;
    assert_eq!(detail.job.association_status, AssociationStatus::Ambiguous);
    assert_eq!(detail.job.observed_status, ObservedStatus::Unknown);
    assert!(detail.generations.is_empty());
    assert_eq!(
        store
            .list_generations(GenerationsQuery {
                association: Some("unassigned".into()),
                ..Default::default()
            })
            .await?
            .items
            .len(),
        2
    );
    Ok(())
}
