use super::{tests::fixture, OperationLogArchive};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use shaula_core::operation_log::*;

#[tokio::test]
async fn invocation_pages_keep_latest_operations_independent_of_page_and_bind_generation(
) -> Result<(), Box<dyn std::error::Error>> {
    let (temporary, store, generation) = fixture().await?;
    let archive =
        OperationLogArchive::open(store, temporary.path().join("logs"), LogConfig::default())
            .await?;
    let mut ids = Vec::new();
    for started_at in 0..5 {
        ids.push(
            archive
                .begin(BeginInvocation {
                    generation_id: generation.clone(),
                    operation: if started_at == 0 { "Create" } else { "Destroy" }.into(),
                    started_at,
                })
                .await?,
        );
    }
    let first = archive
        .list_invocations_page(
            &generation,
            InvocationQuery {
                limit: Some(2),
                cursor: None,
            },
        )
        .await?;
    assert_eq!(
        first.items.iter().map(|v| &v.id).collect::<Vec<_>>(),
        vec![&ids[4], &ids[3]]
    );
    assert_eq!(first.latest_create.as_ref().map(|v| &v.id), Some(&ids[0]));
    assert_eq!(first.latest_destroy.as_ref().map(|v| &v.id), Some(&ids[4]));
    assert!(archive
        .list_invocations_page(
            &uuid::Uuid::new_v4().to_string(),
            InvocationQuery {
                limit: Some(2),
                cursor: first.next_cursor.clone()
            }
        )
        .await
        .is_err());
    let second = archive
        .list_invocations_page(
            &generation,
            InvocationQuery {
                limit: Some(2),
                cursor: first.next_cursor,
            },
        )
        .await?;
    assert_eq!(
        second.items.iter().map(|v| &v.id).collect::<Vec<_>>(),
        vec![&ids[2], &ids[1]]
    );
    assert_eq!(second.latest_destroy.as_ref().map(|v| &v.id), Some(&ids[4]));
    let final_page = archive
        .list_invocations_page(
            &generation,
            InvocationQuery {
                limit: Some(2),
                cursor: second.next_cursor,
            },
        )
        .await?;
    assert_eq!(final_page.items.len(), 1);
    assert_eq!(final_page.items[0].id, ids[0]);
    assert!(final_page.next_cursor.is_none());
    assert!(archive
        .list_invocations_page(
            &generation,
            InvocationQuery {
                limit: Some(201),
                cursor: None
            }
        )
        .await
        .is_err());
    Ok(())
}

#[tokio::test]
async fn metadata_retention_protects_exact_runner_candidates_but_not_a_whole_busy_scope(
) -> Result<(), Box<dyn std::error::Error>> {
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
            started_at: super::now(),
        })
        .await?;
    archive
        .finish(FinishInvocation {
            invocation_id: id.clone(),
            execution_outcome: "failed".into(),
            ended_at: super::now(),
            lost_bytes: 0,
            partial: false,
        })
        .await?;
    let old = super::now() - 100 * 86_400_000;
    let mut record = archive.load(&id).await?;
    record.capture_sealed_at = Some(old);
    archive.save(&record).await?;
    store
        .connection()
        .execute(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "UPDATE runner_generations SET state='Destroyed',updated_at=? WHERE id=?",
            [old.into(), generation.clone().into()],
        ))
        .await?;
    store.connection().execute(Statement::from_sql_and_values(DbBackend::Sqlite,
        "INSERT INTO workflow_generation_identity(generation_id,scope_key,fleet_incarnation,github_runner_id) VALUES(?,'scope','original',7)",[generation.into()])).await?;
    store.connection().execute(Statement::from_sql_and_values(DbBackend::Sqlite,
        "INSERT INTO workflow_jobs(id,scope_key,fleet_key,fleet_incarnation,scale_set_id,protocol_job_id,summary_json,status,created_at,updated_at) VALUES('job','scope','fleet','original',1,'job','{}','completed',1,?)",[super::now().into()])).await?;
    store.connection().execute_unprepared("INSERT INTO workflow_job_observations(id,scope_key,job_record_id,protocol_job_id,runner_request_id,runner_id,data_json,observed_at) VALUES('observation','scope','job','job',1,7,'{}',1)").await?;
    archive.maintenance().await?;
    assert!(archive.load(&id).await.is_ok());
    store
        .connection()
        .execute_unprepared("UPDATE workflow_job_observations SET runner_id=8")
        .await?;
    archive.maintenance().await?;
    assert!(archive.load(&id).await.is_err());
    Ok(())
}
