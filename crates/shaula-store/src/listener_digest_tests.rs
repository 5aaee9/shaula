//! Real pre-Jobs SQLite rows must survive verified re-delivery after migration.
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use sha2::Digest;
use shaula_core::{
    jobs::{JobsQuery, JobsReadPort},
    ports::PollMessage,
    registry::SessionEffectContext,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

// Exact JSON emitted by the pre-Jobs structs, including their field order.
const LEGACY_JSON: &str = concat!(
    r#"{"message_id":1,"statistics":{"total_assigned_jobs":3,"total_registered_runners":0,"total_busy_runners":0,"total_idle_runners":0},"#,
    r#""job_available":[{"runner_request_id":101,"job_id":"job-1"}],"job_assigned":[],"#,
    r#""job_started":[{"runner_request_id":99,"job_id":"job-old","runner_id":2,"runner_name":"runner-2"}],"#,
    r#""job_completed":[{"runner_request_id":98,"job_id":"job-done","runner_id":3,"runner_name":"runner-3"}]}"#,
);

fn redelivery() -> TestResult<PollMessage> {
    let mut message: PollMessage = serde_json::from_str(LEGACY_JSON)?;
    message.job_available[0].metadata.owner_name = Some("example-org".into());
    message.job_available[0].metadata.repository_name = Some("repository".into());
    message.job_available[0].metadata.workflow_run_id = Some(17);
    message.job_started[0].metadata.job_display_name = Some("build".into());
    message.job_completed[0].result = Some("succeeded".into());
    Ok(message)
}

async fn number(store: &crate::Store, sql: &str) -> TestResult<i64> {
    let row = store
        .connection()
        .query_one(Statement::from_string(DbBackend::Sqlite, sql))
        .await?
        .ok_or("missing count")?;
    Ok(row.try_get("", "value")?)
}

async fn legacy_fixture(acked: bool) -> TestResult<(crate::Store, SessionEffectContext)> {
    let (store, context) = crate::tests::listener_messages::ready().await?;
    // Restore the actual m0011 listener table shape before writing the old row.
    // Later unrelated empty tables are immaterial to this forward migration.
    store
        .connection()
        .execute_unprepared(
            "DROP TABLE workflow_job_observations;
         DROP TABLE workflow_jobs;
         DROP TABLE workflow_generation_identity;
         DROP INDEX idx_workflow_generation_page;
         ALTER TABLE listener_messages DROP COLUMN payload_digest_version;
         DELETE FROM seaql_migrations WHERE version='m0012_workflow_jobs';",
        )
        .await?;
    let old_hash = hex::encode(sha2::Sha256::digest(LEGACY_JSON.as_bytes()));
    store.connection().execute(Statement::from_sql_and_values(DbBackend::Sqlite,
        "INSERT INTO listener_messages(fleet_key,epoch,message_id,payload_digest,incarnation,
         fleet_revision,mutation_fence,profile_key,auth_revision,context_json,acked,created_at,updated_at)
         VALUES('fleet',?,1,?,?,?,?,?,?,?,?,20,21)",
        vec![context.epoch.into(),old_hash.into(),context.guard.incarnation.clone().into(),
            context.guard.desired_revision.into(),context.guard.mutation_fence.into(),
            context.auth_context.profile_key.clone().into(),context.auth_context.revision.into(),
            serde_json::to_string(&context.auth_context)?.into(),i64::from(acked).into()])).await?;
    store.connection().execute(Statement::from_sql_and_values(DbBackend::Sqlite,
        "INSERT INTO listener_acquisitions(fleet_key,epoch,message_id,runner_request_id,state,updated_at)
         VALUES('fleet',?,1,101,'Acquired',21)", [context.epoch.into()])).await?;
    store
        .connection()
        .execute(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "INSERT INTO listener_job_observations(fleet_key,epoch,message_id,observation_kind,
         runner_request_id,job_id,runner_name,observed_at)
         VALUES('fleet',?,1,'Started',99,'job-old','runner-2',20)",
            [context.epoch.into()],
        ))
        .await?;
    store.connection().execute_unprepared(
        "INSERT INTO fleet_demand(fleet_key,total_assigned_jobs,updated_at) VALUES('fleet',3,20)
         ON CONFLICT(fleet_key) DO UPDATE SET total_assigned_jobs=3,updated_at=20",
    ).await?;
    store.migrate().await?;
    assert_eq!(
        number(
            &store,
            "SELECT payload_digest_version AS value FROM listener_messages"
        )
        .await?,
        1
    );
    assert_eq!(
        number(&store, "SELECT COUNT(*) AS value FROM workflow_jobs").await?,
        0
    );
    Ok((store, context))
}

#[test]
fn legacy_projection_matches_the_exact_original_struct_encoding() -> TestResult {
    let message = redelivery()?;
    let old_hash = hex::encode(sha2::Sha256::digest(LEGACY_JSON.as_bytes()));
    assert_eq!(super::legacy(&message)?, old_hash);
    assert_ne!(super::current(&message)?, old_hash);
    Ok(())
}

#[tokio::test]
async fn upgrade_redelivery_freezes_metadata_once_without_replaying_lifecycle_facts() -> TestResult
{
    for acked in [false, true] {
        let (store, context) = legacy_fixture(acked).await?;
        let message = redelivery()?;
        // Reading old pending/ACK state does not synthesize new job history.
        let _ = store.listener_pending("fleet", &context).await?;
        assert_eq!(
            number(&store, "SELECT COUNT(*) AS value FROM workflow_jobs").await?,
            0
        );
        for now in [30, 40] {
            let result = store
                .listener_ingest("fleet", &context, &message, now)
                .await?
                .ok_or("current session rejected")?;
            assert_eq!(result.acked, acked);
            assert!(result.pending_request_ids.is_empty());
        }
        assert_eq!(
            number(
                &store,
                "SELECT payload_digest_version AS value FROM listener_messages"
            )
            .await?,
            2
        );
        assert_eq!(
            number(
                &store,
                "SELECT COUNT(*) AS value FROM workflow_job_observations"
            )
            .await?,
            3
        );
        assert_eq!(
            number(
                &store,
                "SELECT MIN(observed_at) AS value FROM workflow_job_observations"
            )
            .await?,
            30
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
                "SELECT COUNT(*) AS value FROM listener_acquisitions WHERE state='Acquired'"
            )
            .await?,
            1
        );
        assert_eq!(
            number(&store, "SELECT updated_at AS value FROM fleet_demand").await?,
            20
        );
        assert_eq!(
            number(&store, "SELECT updated_at AS value FROM listener_messages").await?,
            21
        );
        let page = store.list_jobs(JobsQuery::default()).await?;
        assert_eq!(page.items.len(), 3);
        let job = page
            .items
            .iter()
            .find(|job| job.protocol_job_id == "job-1")
            .ok_or("missing enriched job")?;
        assert_eq!(job.metadata.workflow_run_id, Some(17));
        let mut changed = message;
        changed.job_available[0].metadata.workflow_run_id = Some(18);
        assert!(store
            .listener_ingest("fleet", &context, &changed, 50)
            .await
            .is_err());
        assert_eq!(
            number(
                &store,
                "SELECT COUNT(*) AS value FROM workflow_job_observations"
            )
            .await?,
            3
        );
    }
    Ok(())
}

#[tokio::test]
async fn legacy_identity_or_statistics_changes_remain_rejected_before_promotion() -> TestResult {
    let (store, context) = legacy_fixture(false).await?;
    for field in 0..3 {
        let mut changed = redelivery()?;
        match field {
            0 => changed.job_available[0].runner_request_id += 1,
            1 => changed.job_started[0].runner_id += 1,
            _ => changed.statistics.total_assigned_jobs += 1,
        }
        assert!(store
            .listener_ingest("fleet", &context, &changed, 30)
            .await
            .is_err());
    }
    assert_eq!(
        number(
            &store,
            "SELECT payload_digest_version AS value FROM listener_messages"
        )
        .await?,
        1
    );
    assert_eq!(
        number(&store, "SELECT COUNT(*) AS value FROM workflow_jobs").await?,
        0
    );
    Ok(())
}

#[tokio::test]
async fn failed_metadata_commit_keeps_the_legacy_digest_retryable() -> TestResult {
    let (store, context) = legacy_fixture(false).await?;
    store
        .connection()
        .execute_unprepared(
            "CREATE TRIGGER reject_jobs BEFORE INSERT ON workflow_jobs BEGIN
         SELECT RAISE(ABORT,'test metadata unavailable'); END;",
        )
        .await?;
    let message = redelivery()?;
    assert!(store
        .listener_ingest("fleet", &context, &message, 30)
        .await
        .is_err());
    assert_eq!(
        number(
            &store,
            "SELECT payload_digest_version AS value FROM listener_messages"
        )
        .await?,
        1
    );
    assert_eq!(
        number(&store, "SELECT COUNT(*) AS value FROM workflow_jobs").await?,
        0
    );
    store
        .connection()
        .execute_unprepared("DROP TRIGGER reject_jobs")
        .await?;
    assert!(store
        .listener_ingest("fleet", &context, &message, 40)
        .await?
        .is_some());
    Ok(())
}

#[tokio::test]
async fn stale_session_cannot_promote_legacy_metadata() -> TestResult {
    let (store, mut stale) = legacy_fixture(true).await?;
    stale.epoch += 1;
    assert!(store
        .listener_ingest("fleet", &stale, &redelivery()?, 30)
        .await?
        .is_none());
    assert_eq!(
        number(
            &store,
            "SELECT payload_digest_version AS value FROM listener_messages"
        )
        .await?,
        1
    );
    assert_eq!(
        number(&store, "SELECT COUNT(*) AS value FROM workflow_jobs").await?,
        0
    );
    Ok(())
}
