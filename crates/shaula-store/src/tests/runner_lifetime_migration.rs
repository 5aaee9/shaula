use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use shaula_core::runner_lifetime::RunnerLifetimeStore;
use shaula_store_migration::{Migrator, MigratorTrait};

use super::forgejo_pool_support::TestResult;
use crate::{registry_impl::SqliteControlPlane, Store};

#[tokio::test]
async fn lifetime_migration_failure_rolls_back_checkpoints_and_history() -> TestResult {
    let directory = tempfile::tempdir()?;
    let store = Store::open(&directory.path().join("ledger.db")).await?;
    Migrator::up(store.connection(), Some(19)).await?;
    store
        .connection()
        .execute_unprepared(
            "INSERT INTO runner_generations
         (id, fleet_key, runner_name, generation_name, fleet_revision, template_profile_key,
          template_revision, template_artifact_digest, attestation_id, inputs_digest,
          state, workspace_path, shaula_result_json, created_at, updated_at)
         VALUES ('gen', 'fleet', 'runner', 'gen', 1, 'profile', 1, 'digest', 'att', 'inputs',
                 'Busy', 'work', '{}', 10, 9000);
         CREATE TRIGGER reject_lifetime BEFORE UPDATE ON runner_generations
         BEGIN SELECT RAISE(ABORT, 'injected migration failure'); END;",
        )
        .await?;
    assert!(store.migrate().await.is_err());
    use super::template_source_migration_support::scalar;
    assert_eq!(scalar(&store, "SELECT COUNT(*) AS value FROM pragma_table_info('runner_generations') WHERE name IN ('provisioned_at', 'expiry_requested_at', 'resources_destroyed_at')").await?, 0);
    assert_eq!(
        scalar(&store, "SELECT COUNT(*) AS value FROM seaql_migrations").await?,
        19
    );
    assert_eq!(
        scalar(
            &store,
            "SELECT updated_at AS value FROM runner_generations WHERE id = 'gen'"
        )
        .await?,
        9000
    );
    store
        .connection()
        .execute_unprepared("DROP TRIGGER reject_lifetime")
        .await?;
    store.migrate().await?;
    let plane = SqliteControlPlane::new(store, directory.path().join("artifacts"));
    assert_eq!(
        plane.generation_lifetime("gen").await?.provisioned_at,
        Some(9000)
    );
    Ok(())
}

#[tokio::test]
async fn lifetime_migration_backfills_once_without_inventing_failed_create_success() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("ledger.db");
    let store = Store::open(&path).await?;
    Migrator::up(store.connection(), Some(19)).await?;
    for (id, result) in [
        ("completed", Some("{}")),
        ("observed", Some("{}")),
        ("failed", None),
    ] {
        store.connection().execute(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
            "INSERT INTO runner_generations
             (id, fleet_key, runner_name, generation_name, fleet_revision, template_profile_key,
              template_revision, template_artifact_digest, attestation_id, inputs_digest,
              state, workspace_path, shaula_result_json, created_at, updated_at)
             VALUES (?, 'fleet', ?, ?, 1, 'profile', 1, 'digest', 'att', 'inputs', 'Busy', 'work', ?, 10, 9000)",
            [id.into(), id.into(), id.into(), result.into()])).await?;
    }
    store.connection().execute_unprepared(
        "INSERT INTO runner_operations (id, generation_id, kind, state, attempts, created_at, updated_at)
         VALUES ('create', 'completed', 'Create', 'Succeeded', 1, 10, 1000),
                ('recovered', 'completed', 'Create', 'Completed', 1, 10, 2000),
                ('failed-create', 'failed', 'Create', 'ApplyStarting', 1, 10, 1000)",
    ).await?;
    store.migrate().await?;
    let plane = SqliteControlPlane::new(store.clone(), directory.path().join("artifacts"));
    assert_eq!(
        plane.generation_lifetime("completed").await?.provisioned_at,
        Some(1000)
    );
    assert_eq!(
        plane.generation_lifetime("observed").await?.provisioned_at,
        Some(9000)
    );
    assert_eq!(
        plane.generation_lifetime("failed").await?.provisioned_at,
        None
    );
    store
        .connection()
        .execute_unprepared("UPDATE runner_generations SET updated_at = 99000")
        .await?;
    let reopened = Store::open(&path).await?;
    reopened.migrate().await?;
    let plane = SqliteControlPlane::new(reopened, directory.path().join("artifacts"));
    assert_eq!(
        plane.generation_lifetime("completed").await?.provisioned_at,
        Some(1000)
    );
    assert_eq!(
        plane.generation_lifetime("observed").await?.provisioned_at,
        Some(9000)
    );
    assert_eq!(
        plane.generation_lifetime("failed").await?.provisioned_at,
        None
    );
    assert_eq!(
        plane
            .generation_lifetime("completed")
            .await?
            .expiry_requested_at,
        None
    );
    Ok(())
}
