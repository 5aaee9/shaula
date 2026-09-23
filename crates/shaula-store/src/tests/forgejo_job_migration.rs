use super::{forgejo_pool_support::TestResult, template_source_migration_support::scalar};
use crate::Store;
use sea_orm::ConnectionTrait;
use shaula_store_migration::{Migrator, MigratorTrait};

#[tokio::test]
async fn enrichment_migration_preserves_polls_and_rolls_back_with_its_version_row() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("jobs.db");
    let store = Store::open(&path).await?;
    Migrator::up(store.connection(), Some(21)).await?;
    store.connection().execute_unprepared(
        "INSERT INTO forgejo_job_polls(scope_key,observed_at,attempted_at,failed) VALUES('scope',100,200,1);
         CREATE TRIGGER reject_enrichment_version BEFORE INSERT ON seaql_migrations
         WHEN NEW.version='m0022_forgejo_job_enrichment'
         BEGIN SELECT RAISE(ABORT,'injected version commit failure'); END;",
    ).await?;
    assert!(store.migrate().await.is_err());
    assert_eq!(scalar(&store, "SELECT COUNT(*) AS value FROM pragma_table_info('forgejo_job_polls') WHERE name='enrichment_attempted_at'").await?, 0);
    assert_eq!(
        scalar(&store, "SELECT COUNT(*) AS value FROM seaql_migrations").await?,
        21
    );
    store
        .connection()
        .execute_unprepared("DROP TRIGGER reject_enrichment_version")
        .await?;
    store.migrate().await?;
    for (column, expected) in [
        ("observed_at", 100),
        ("attempted_at", 200),
        ("failed", 1),
        ("enrichment_attempted_at", 0),
    ] {
        assert_eq!(
            scalar(
                &store,
                &format!("SELECT {column} AS value FROM forgejo_job_polls WHERE scope_key='scope'")
            )
            .await?,
            expected
        );
    }
    store
        .connection()
        .execute_unprepared("UPDATE forgejo_job_polls SET enrichment_attempted_at=300")
        .await?;
    let reopened = Store::open(&path).await?;
    reopened.migrate().await?;
    assert_eq!(scalar(&reopened, "SELECT enrichment_attempted_at AS value FROM forgejo_job_polls WHERE scope_key='scope'").await?, 300);
    Ok(())
}
