//! Spec 0001 GitHub lifecycle: observed job starts/completions drive the
//! exact Generation Idle -> Busy -> Retiring inside the message transaction.
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use shaula_core::lifecycle::GenerationState;
use shaula_core::ports::{JobCompletedMessage, JobStartedMessage, PollMessage};
use shaula_core::registry::GenerationRecord;

use super::listener_messages::ready;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

async fn generation(
    store: &crate::Store,
    id: &str,
    fleet: &str,
    runner_id: i64,
    state: &str,
) -> TestResult {
    store
        .generation_insert(GenerationRecord {
            id: id.into(),
            fleet_key: fleet.into(),
            runner_name: format!("runner-{id}"),
            generation_name: format!("s-{id}"),
            fleet_revision: 1,
            template_profile_key: "tpl".into(),
            pool_member_key: None,
            template_revision: 1,
            template_artifact_digest: "sha256:artifact".into(),
            attestation_id: "att".into(),
            inputs_digest: "sha256:inputs".into(),
            state: GenerationState::Creating,
            github_runner_id: None,
            workspace_path: format!("ws-{id}"),
            created_at: 1,
            updated_at: 1,
        })
        .await?;
    store
        .connection()
        .execute(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "UPDATE runner_generations SET state=?, github_runner_id=? WHERE id=?",
            [state.into(), runner_id.into(), id.into()],
        ))
        .await?;
    Ok(())
}

async fn state(store: &crate::Store, id: &str) -> TestResult<String> {
    Ok(store.generation_get(id).await?.ok_or("absent")?.state)
}

fn message(id: i64, started: &[(i64, &str)], completed: &[(i64, &str)]) -> PollMessage {
    PollMessage {
        message_id: id,
        statistics: Default::default(),
        job_available: vec![],
        job_assigned: vec![],
        job_started: started
            .iter()
            .map(|(runner_id, name)| JobStartedMessage {
                runner_request_id: 0,
                job_id: format!("job-{runner_id}"),
                runner_id: *runner_id,
                runner_name: (*name).into(),
                metadata: Default::default(),
            })
            .collect(),
        job_completed: completed
            .iter()
            .map(|(runner_id, name)| JobCompletedMessage {
                runner_request_id: 0,
                job_id: format!("job-{runner_id}"),
                runner_id: *runner_id,
                runner_name: (*name).into(),
                metadata: Default::default(),
                result: Some("succeeded".into()),
            })
            .collect(),
    }
}

#[tokio::test]
async fn observed_start_makes_the_exact_generation_busy_then_completion_retires_it() -> TestResult {
    let (store, context) = ready().await?;
    generation(&store, "idle", "fleet", 7, "Idle").await?;
    generation(&store, "waiting", "fleet", 8, "WaitingOnline").await?;
    generation(&store, "renamed", "fleet", 9, "Idle").await?;
    generation(&store, "other", "other-fleet", 7, "Idle").await?;
    generation(&store, "retiring", "fleet", 10, "Retiring").await?;
    let started = message(
        1,
        &[
            (7, "runner-idle"),
            (8, "runner-waiting"),
            (9, "not-this-runner"),
            (10, "runner-retiring"),
        ],
        &[],
    );
    store
        .listener_ingest("fleet", &context, &started, 20)
        .await?
        .ok_or("stale")?;
    assert_eq!(state(&store, "idle").await?, "Busy");
    assert_eq!(
        state(&store, "waiting").await?,
        "Busy",
        "a started job proves online"
    );
    assert_eq!(
        state(&store, "renamed").await?,
        "Idle",
        "runner name must match"
    );
    assert_eq!(state(&store, "other").await?, "Idle", "fleet must match");
    assert_eq!(
        state(&store, "retiring").await?,
        "Retiring",
        "never rewinds"
    );
    // A redelivered start is idempotent; a completion retires the runner.
    store
        .listener_ingest("fleet", &context, &started, 21)
        .await?
        .ok_or("stale")?;
    assert_eq!(state(&store, "idle").await?, "Busy");
    store
        .listener_ingest(
            "fleet",
            &context,
            &message(2, &[], &[(7, "runner-idle")]),
            30,
        )
        .await?
        .ok_or("stale")?;
    assert_eq!(state(&store, "idle").await?, "Retiring");
    assert_eq!(state(&store, "waiting").await?, "Busy");
    Ok(())
}

#[tokio::test]
async fn start_and_completion_in_one_message_end_retiring() -> TestResult {
    let (store, context) = ready().await?;
    generation(&store, "quick", "fleet", 7, "Idle").await?;
    store
        .listener_ingest(
            "fleet",
            &context,
            &message(1, &[(7, "runner-quick")], &[(7, "runner-quick")]),
            20,
        )
        .await?
        .ok_or("stale")?;
    assert_eq!(state(&store, "quick").await?, "Retiring");
    Ok(())
}
