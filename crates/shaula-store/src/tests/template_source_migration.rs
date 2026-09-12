use sea_orm::ConnectionTrait;
use shaula_core::registry::ControlPlaneStore;

use super::template_source_migration_support::{
    restored, retained_snapshot, scalar, Fixture, TestResult,
};
use crate::registry_impl::SqliteControlPlane;

#[tokio::test]
async fn migration_backfills_only_unique_exact_content_with_known_platform() -> TestResult {
    let fixture = Fixture::legacy().await?;
    let before = retained_snapshot(&fixture.store).await?;
    fixture.store.migrate().await?;
    assert_eq!(retained_snapshot(&fixture.store).await?, before);
    for key in ["unique", "second-unique"] {
        let row = fixture
            .store
            .template_revision_get(key, 1)
            .await?
            .ok_or("revision missing")?;
        assert_eq!(row.source_key.as_deref(), Some("docker-legacy"));
    }
    for key in [
        "ambiguous",
        "absent",
        "wrong-engine",
        "wrong-platform",
        "unknown-platform",
        "empty-platform",
    ] {
        let row = fixture
            .store
            .template_revision_get(key, 1)
            .await?
            .ok_or("revision missing")?;
        assert_eq!(row.source_key, None, "{key} must not infer a source");
    }
    let plane = SqliteControlPlane::new(fixture.store.clone(), fixture.temp.path().join("cache"));
    assert_eq!(
        plane.template_source_get("docker-legacy").await?,
        Some(fixture.unique)
    );
    assert!(plane.template_source_get("missing").await?.is_none());
    assert_eq!(
        plane
            .template_revision_get("unique", 1)
            .await?
            .ok_or("revision missing")?
            .source_key
            .as_deref(),
        Some("docker-legacy")
    );
    Ok(())
}

#[tokio::test]
async fn migrated_sources_survive_catalog_removal_restart_and_database_restore() -> TestResult {
    let fixture = Fixture::legacy().await?;
    let before = retained_snapshot(&fixture.store).await?;
    let old_backup = fixture.backup("before-upgrade.db").await?;
    fixture.store.migrate().await?;
    let upgraded_backup = fixture.backup("after-upgrade.db").await?;
    fixture.store.template_sources_replace(&[], 3).await?;
    fixture.store.migrate().await?;
    assert!(fixture.store.template_sources().await?.is_empty());
    assert_eq!(retained_snapshot(&fixture.store).await?, before);
    let reopened = restored(&fixture.path).await?;
    assert_eq!(
        reopened
            .template_revision_get("unique", 1)
            .await?
            .ok_or("revision missing")?
            .source_key
            .as_deref(),
        Some("docker-legacy")
    );
    // A later catalog with a single matching source cannot retroactively associate
    // the formerly ambiguous revision or rename an already recorded old source.
    let mut current = fixture.unique.clone();
    current.key = "docker".into();
    let ambiguous = fixture
        .store
        .template_revision_get("ambiguous", 1)
        .await?
        .ok_or("revision missing")?;
    let mut now_unique = current.clone();
    now_unique.key = "now-unambiguous".into();
    now_unique.artifact_digest = ambiguous.artifact_digest;
    fixture
        .store
        .template_sources_replace(&[current, now_unique], 4)
        .await?;
    fixture.store.migrate().await?;
    assert_eq!(
        fixture
            .store
            .template_revision_get("ambiguous", 1)
            .await?
            .ok_or("revision missing")?
            .source_key,
        None
    );
    assert_eq!(
        fixture
            .store
            .template_revision_get("unique", 1)
            .await?
            .ok_or("revision missing")?
            .source_key
            .as_deref(),
        Some("docker-legacy")
    );
    for backup in [old_backup, upgraded_backup] {
        let restored = restored(&backup).await?;
        assert_eq!(retained_snapshot(&restored).await?, before);
        assert_eq!(
            restored
                .template_revision_get("unique", 1)
                .await?
                .ok_or("revision missing")?
                .source_key
                .as_deref(),
            Some("docker-legacy")
        );
    }
    Ok(())
}

#[tokio::test]
async fn source_backfill_failure_rolls_back_schema_data_and_migration_history() -> TestResult {
    let fixture = Fixture::legacy().await?;
    let before = retained_snapshot(&fixture.store).await?;
    fixture
        .store
        .connection()
        .execute_unprepared(
            "CREATE TRIGGER reject_source_backfill BEFORE UPDATE ON template_profile_revisions
         WHEN OLD.profile_key = 'second-unique'
         BEGIN SELECT RAISE(ABORT, 'injected migration failure'); END;",
        )
        .await?;
    assert!(fixture.store.migrate().await.is_err());
    assert_eq!(retained_snapshot(&fixture.store).await?, before);
    assert_eq!(scalar(&fixture.store, "SELECT COUNT(*) AS value FROM pragma_table_info('template_profile_revisions') WHERE name='source_key'").await?, 0);
    assert_eq!(
        scalar(
            &fixture.store,
            "SELECT COUNT(*) AS value FROM seaql_migrations"
        )
        .await?,
        14
    );
    fixture
        .store
        .connection()
        .execute_unprepared("DROP TRIGGER reject_source_backfill")
        .await?;
    fixture.store.migrate().await?;
    assert_eq!(
        scalar(
            &fixture.store,
            "SELECT COUNT(*) AS value FROM seaql_migrations"
        )
        .await?,
        17
    );
    assert_eq!(
        fixture
            .store
            .template_revision_get("unique", 1)
            .await?
            .ok_or("revision missing")?
            .source_key
            .as_deref(),
        Some("docker-legacy")
    );
    assert_eq!(retained_snapshot(&fixture.store).await?, before);
    Ok(())
}
