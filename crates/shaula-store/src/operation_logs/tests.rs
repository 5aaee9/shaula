use super::*;
use shaula_core::operation_log::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;

pub(super) async fn fixture(
) -> Result<(tempfile::TempDir, Store, String), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let store = Store::open(&temporary.path().join("test.db")).await?;
    store.migrate().await?;
    store.connection().execute_unprepared("INSERT INTO fleets(key,incarnation,desired_revision,observed_revision,mutation_fence,deletion_marker,phase,tombstone,created_at,updated_at) VALUES('fleet','incarnation',1,1,1,0,'Active',0,1,1)").await?;
    let generation = uuid::Uuid::new_v4().to_string();
    store.connection().execute(Statement::from_sql_and_values(DbBackend::Sqlite,
        "INSERT INTO runner_generations(id,fleet_key,runner_name,generation_name,fleet_revision,template_profile_key,template_revision,template_artifact_digest,attestation_id,inputs_digest,state,workspace_path,created_at,updated_at) VALUES(?,'fleet','runner','generation',1,'template',1,'artifact','activation','inputs','Creating','workspace',1,1)", [generation.clone().into()]
    )).await?;
    Ok((temporary, store, generation))
}

fn chunk(id: &str, sequence: u64, text: &str) -> AppendLog {
    AppendLog {
        invocation_id: id.into(),
        command_ordinal: 0,
        phase: "apply".into(),
        stream: "stdout".into(),
        sequence,
        text: text.into(),
        observed_at: now(),
        withheld: false,
    }
}

async fn finish(archive: &OperationLogArchive, id: &str, outcome: &str) -> CoreResult<()> {
    archive
        .finish(FinishInvocation {
            invocation_id: id.into(),
            execution_outcome: outcome.into(),
            ended_at: now(),
            lost_bytes: 0,
            partial: false,
        })
        .await
}

#[tokio::test]
async fn destroy_attempts_survive_restart_and_replays_are_checked() -> TestResult {
    let (temporary, store, generation) = fixture().await?;
    let root = temporary.path().join("logs");
    let archive =
        OperationLogArchive::open(store.clone(), root.clone(), LogConfig::default()).await?;
    let first = archive
        .begin(BeginInvocation {
            generation_id: generation.clone(),
            operation: "Destroy".into(),
            started_at: now(),
        })
        .await?;
    archive
        .append(chunk(&first, 0, "docker_container.runner: Destroying...\n"))
        .await?;
    let mut changed_source = chunk(&first, 0, "docker_container.runner: Destroying...\n");
    changed_source.stream = "stderr".into();
    assert!(archive.append(changed_source).await.is_err());
    archive
        .append(chunk(&first, 0, "docker_container.runner: Destroying...\n"))
        .await?;
    assert!(archive.append(chunk(&first, 0, "different")).await.is_err());
    finish(&archive, &first, "failed").await?;
    let second = archive
        .begin(BeginInvocation {
            generation_id: generation.clone(),
            operation: "Destroy".into(),
            started_at: now(),
        })
        .await?;
    archive
        .append(chunk(
            &second,
            0,
            "docker_container.runner: Destruction complete after 1s\n",
        ))
        .await?;
    finish(&archive, &second, "succeeded").await?;
    drop(archive);
    let archive = OperationLogArchive::open(store, root, LogConfig::default()).await?;
    let attempts = archive.list_invocations(&generation).await?;
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].id, second);
    assert!(attempts.iter().any(|v| v.execution_outcome == "failed"));
    assert_eq!(
        archive
            .read_page(&first, LogQuery::default())
            .await?
            .entries
            .len(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn incarnation_uses_frozen_identity_and_unknown_is_not_current_fleet() -> TestResult {
    let (temporary, store, generation) = fixture().await?;
    let archive = OperationLogArchive::open(
        store.clone(),
        temporary.path().join("logs"),
        LogConfig::default(),
    )
    .await?;
    let request = BeginInvocation {
        generation_id: generation.clone(),
        operation: "Create".into(),
        started_at: now(),
    };
    let unknown = archive.begin(request.clone()).await?;
    assert_eq!(archive.load(&unknown).await?.fleet_incarnation, None);
    store.connection().execute(Statement::from_sql_and_values(DbBackend::Sqlite,
        "INSERT INTO workflow_generation_identity(generation_id,scope_key,fleet_incarnation) VALUES(?,'scope','original')", [generation.into()])).await?;
    store
        .connection()
        .execute_unprepared("UPDATE fleets SET incarnation='replacement'")
        .await?;
    let original = archive.begin(request).await?;
    assert_eq!(
        archive.load(&original).await?.fleet_incarnation.as_deref(),
        Some("original")
    );
    Ok(())
}

#[tokio::test]
async fn metadata_survives_recent_destruction_and_apply_projection_seals_independently(
) -> TestResult {
    let (temporary, store, generation) = fixture().await?;
    let archive = OperationLogArchive::open(
        store.clone(),
        temporary.path().join("logs"),
        LogConfig::default(),
    )
    .await?;
    let id = archive
        .begin(BeginInvocation {
            generation_id: generation.clone(),
            operation: "Create".into(),
            started_at: now(),
        })
        .await?;
    archive
        .append(chunk(
            &id,
            0,
            "Apply complete! Resources: 1 added, 0 changed, 0 destroyed.\n",
        ))
        .await?;
    archive
        .command(LogCommand {
            invocation_id: id.clone(),
            ordinal: 0,
            phase: "apply".into(),
            started_at: now(),
            ended_at: Some(now()),
            exit_code: Some(0),
            termination: "exited".into(),
            effect_attempt_id: None,
            capture_partial: false,
            lost_bytes: 0,
        })
        .await?;
    assert_eq!(archive.setup_projection(&generation).await?.status, "ready");
    assert!(archive.load(&id).await?.capture_sealed_at.is_none());
    let mut record = archive.load(&id).await?;
    record.policy_version = "retired-policy".into();
    archive.save(&record).await?;
    assert_eq!(
        archive.setup_projection(&generation).await?.status,
        "withheld"
    );
    finish(&archive, &id, "succeeded").await?;
    let mut record = archive.load(&id).await?;
    record.capture_sealed_at = Some(now() - 100 * 86_400_000);
    archive.save(&record).await?;
    store
        .connection()
        .execute(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "UPDATE runner_generations SET state='Destroyed',updated_at=? WHERE id=?",
            [now().into(), generation.clone().into()],
        ))
        .await?;
    archive.maintenance().await?;
    assert_eq!(archive.load(&id).await?.capture_status, "expired");
    store
        .connection()
        .execute(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "UPDATE runner_generations SET updated_at=? WHERE id=?",
            [(now() - 100 * 86_400_000).into(), generation.into()],
        ))
        .await?;
    archive.maintenance().await?;
    assert!(archive.load(&id).await.is_err());
    Ok(())
}

#[tokio::test]
async fn limit_keeps_head_and_tail_and_cursor_cannot_cross_invocations() -> TestResult {
    let (temporary, store, generation) = fixture().await?;
    let archive = OperationLogArchive::open(
        store,
        temporary.path().join("logs"),
        LogConfig {
            invocation_bytes: 128 * 1024,
            ..LogConfig::default()
        },
    )
    .await?;
    let first = archive
        .begin(BeginInvocation {
            generation_id: generation.clone(),
            operation: "Create".into(),
            started_at: now(),
        })
        .await?;
    for sequence in 0..4 {
        archive
            .append(chunk(&first, sequence, &"a".repeat(65536)))
            .await?;
    }
    let page = archive
        .read_page(
            &first,
            LogQuery {
                limit_bytes: Some(65536),
                ..LogQuery::default()
            },
        )
        .await?;
    assert!(page.has_gap);
    assert_eq!(page.entries[0].sequence, 0);
    let second = archive
        .begin(BeginInvocation {
            generation_id: generation,
            operation: "Destroy".into(),
            started_at: now(),
        })
        .await?;
    assert!(archive
        .read_page(
            &second,
            LogQuery {
                cursor: page.next_cursor,
                ..LogQuery::default()
            }
        )
        .await
        .is_err());
    let record = archive.load(&first).await?;
    assert_eq!(record.retained_bytes, 128 * 1024);
    assert_eq!(record.lost_bytes, 128 * 1024);
    Ok(())
}

#[tokio::test]
async fn interrupted_writer_is_not_a_success_and_missing_file_is_unavailable() -> TestResult {
    let (temporary, store, generation) = fixture().await?;
    let root = temporary.path().join("logs");
    let archive =
        OperationLogArchive::open(store.clone(), root.clone(), LogConfig::default()).await?;
    let id = archive
        .begin(BeginInvocation {
            generation_id: generation,
            operation: "Create".into(),
            started_at: now(),
        })
        .await?;
    archive
        .append(chunk(
            &id,
            0,
            "Apply complete! Resources: 1 added, 0 changed, 0 destroyed.\n",
        ))
        .await?;
    drop(archive);
    let archive = OperationLogArchive::open(store, root.clone(), LogConfig::default()).await?;
    let record = archive.load(&id).await?;
    assert_eq!(record.execution_outcome, "unknown");
    assert_eq!(record.capture_status, "partial");
    tokio::fs::remove_file(root.join(&id).join("00000000000000000000.log")).await?;
    let page = archive.read_page(&id, LogQuery::default()).await?;
    assert_eq!(page.capture_status, "unavailable");
    assert!(page.entries.is_empty());
    Ok(())
}

#[tokio::test]
async fn global_quota_counts_orphans_and_never_evicts_an_active_capture() -> TestResult {
    let (temporary, store, generation) = fixture().await?;
    let root = temporary.path().join("logs");
    let orphan_dir = root.join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&orphan_dir)?;
    let orphan = orphan_dir.join("00000000000000000000.log");
    std::fs::write(&orphan, vec![b'x'; 65536])?;
    let archive = OperationLogArchive::open(
        store,
        root,
        LogConfig {
            invocation_bytes: 128 * 1024,
            quota_bytes: 128 * 1024,
            ..LogConfig::default()
        },
    )
    .await?;
    let id = archive
        .begin(BeginInvocation {
            generation_id: generation,
            operation: "Create".into(),
            started_at: now(),
        })
        .await?;
    archive.append(chunk(&id, 0, &"a".repeat(65536))).await?;
    archive.append(chunk(&id, 1, &"b".repeat(65536))).await?;
    let record = archive.load(&id).await?;
    assert_eq!(record.retained_bytes, 65536);
    assert_eq!(record.lost_bytes, 65536);
    assert!(record.capture_sealed_at.is_none());
    let modified = std::time::SystemTime::now() - std::time::Duration::from_secs(600);
    std::fs::File::options()
        .write(true)
        .open(&orphan)?
        .set_times(std::fs::FileTimes::new().set_modified(modified))?;
    archive.maintenance().await?;
    assert!(!orphan.exists());
    archive.append(chunk(&id, 2, &"c".repeat(65536))).await?;
    assert_eq!(archive.load(&id).await?.retained_bytes, 128 * 1024);
    Ok(())
}
