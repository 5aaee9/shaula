//! Ownership markers survive recovery without promoting unowned candidates.

use sea_orm::{ConnectionTrait, DbBackend, Statement};
use shaula_core::registry::ScaleSetRow;
use shaula_store_migration::{Migrator, MigratorTrait};

use crate::Store;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[tokio::test]
async fn scale_set_ownership_marker_requires_proof_and_survives_only_same_identity() -> TestResult {
    let temp = tempfile::tempdir()?;
    let store = Store::open(&temp.path().join("ownership.db")).await?;
    store.migrate().await?;

    let mut candidate = row("AccessBlocked");
    candidate.owned_scale_set_id = Some(42);
    store.scale_set_upsert(candidate).await?;
    assert_eq!(owned(&store).await?, None, "caller cannot assert ownership");
    store.scale_set_upsert(row("Adopted")).await?;
    assert_eq!(owned(&store).await?, Some(42));

    for state in [
        "LabelsUpdating",
        "LabelsUpdateUncertain",
        "AccessBlocked",
        "UnknownRemoteRunner",
    ] {
        store.scale_set_upsert(row(state)).await?;
        assert_eq!(owned(&store).await?, Some(42), "proof survives {state}");
    }
    for state in [
        "Unbound",
        "ScaleSetMissingWithResources",
        "ScaleSetCreateStarting",
        "Other",
    ] {
        store.scale_set_upsert(row("Adopted")).await?;
        let mut reopening = row(state);
        reopening.scale_set_id = None;
        store.scale_set_upsert(reopening).await?;
        let observed = store.scale_set_get("fleet").await?.ok_or("missing row")?;
        assert_eq!(observed.scale_set_id, Some(42), "diagnostic ID retained");
        assert_eq!(
            observed.owned_scale_set_id, None,
            "proof cleared for {state}"
        );
    }

    let mut other_id = row("AccessBlocked");
    other_id.scale_set_id = Some(43);
    let mut other_fingerprint = row("AccessBlocked");
    other_fingerprint.fingerprint = "different target".into();
    let mut other_name = row("AccessBlocked");
    other_name.name = "different name".into();
    let mut other_group = row("AccessBlocked");
    other_group.runner_group = "different group".into();
    for changed in [other_id, other_fingerprint, other_name, other_group] {
        store.scale_set_upsert(row("Adopted")).await?;
        store.scale_set_upsert(changed).await?;
        assert_eq!(owned(&store).await?, None, "proof cannot cross identity");
    }
    for invalid in [None, Some(0), Some(-1)] {
        let mut adopted = row("Adopted");
        adopted.scale_set_id = invalid;
        store.scale_set_upsert(adopted).await?;
        assert_eq!(
            owned(&store).await?,
            None,
            "proof requires a positive explicit ID"
        );
    }
    Ok(())
}

#[tokio::test]
async fn scale_set_ownership_migration_backfills_only_adopted_positive_ids() -> TestResult {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("legacy.db");
    let store = Store::open(&path).await?;
    Migrator::up(store.connection(), Some(15)).await?;
    let cases = [
        ("adopted", "Adopted", Some(42)),
        ("blocked", "AccessBlocked", Some(42)),
        ("unknown", "UnknownRemoteRunner", Some(42)),
        ("pending", "ScaleSetCreateStarting", Some(42)),
        ("unbound", "Unbound", Some(42)),
        ("missing", "ScaleSetMissingWithResources", Some(42)),
        ("zero", "Adopted", Some(0)),
        ("negative", "Adopted", Some(-1)),
        ("null", "Adopted", None),
    ];
    for (key, state, id) in cases {
        legacy_row(&store, key, state, id).await?;
    }
    store.migrate().await?;
    let reopened = Store::open(&path).await?;
    reopened.migrate().await?;
    for (key, state, id) in cases {
        let actual = reopened
            .scale_set_get(key)
            .await?
            .ok_or("missing legacy row")?;
        assert_eq!(actual.scale_set_id, id);
        assert_eq!(actual.state, state);
        assert_eq!(actual.name, "runner-set");
        assert_eq!(actual.runner_group, "Default");
        assert_eq!(actual.fingerprint, "sha256:identity");
        assert_eq!(actual.attempt_id.as_deref(), Some("attempt"));
        assert_eq!(actual.created_at, 1);
        assert_eq!(actual.updated_at, 2);
        assert_eq!(actual.owned_scale_set_id, (key == "adopted").then_some(42));
    }
    Ok(())
}

#[tokio::test]
async fn scale_set_ownership_migration_failure_rolls_back_schema_and_retries() -> TestResult {
    let temp = tempfile::tempdir()?;
    let store = Store::open(&temp.path().join("failure.db")).await?;
    Migrator::up(store.connection(), Some(15)).await?;
    legacy_row(&store, "fleet", "Adopted", Some(42)).await?;
    store
        .connection()
        .execute_unprepared(
            "CREATE TRIGGER reject_ownership BEFORE UPDATE ON scale_set_state
         BEGIN SELECT RAISE(ABORT, 'injected ownership migration failure'); END;",
        )
        .await?;
    assert!(store.migrate().await.is_err());
    assert_eq!(scalar(&store, "SELECT COUNT(*) AS value FROM pragma_table_info('scale_set_state') WHERE name='owned_scale_set_id'").await?, 0);
    assert_eq!(
        scalar(&store, "SELECT COUNT(*) AS value FROM seaql_migrations").await?,
        15
    );
    assert_eq!(
        scalar(
            &store,
            "SELECT scale_set_id AS value FROM scale_set_state WHERE fleet_key='fleet'"
        )
        .await?,
        42
    );
    store
        .connection()
        .execute_unprepared("DROP TRIGGER reject_ownership")
        .await?;
    store.migrate().await?;
    assert_eq!(owned(&store).await?, Some(42));
    assert_eq!(
        scalar(&store, "SELECT COUNT(*) AS value FROM seaql_migrations").await?,
        17
    );
    Ok(())
}

fn row(state: &str) -> ScaleSetRow {
    ScaleSetRow {
        fleet_key: "fleet".into(),
        scale_set_id: Some(42),
        owned_scale_set_id: None,
        name: "runner-set".into(),
        runner_group: "Default".into(),
        fingerprint: "sha256:identity".into(),
        state: state.into(),
        attempt_id: Some("attempt".into()),
        now: 1,
    }
}

async fn owned(store: &Store) -> TestResult<Option<i64>> {
    Ok(store
        .scale_set_get("fleet")
        .await?
        .ok_or("missing row")?
        .owned_scale_set_id)
}

async fn legacy_row(store: &Store, key: &str, state: &str, id: Option<i64>) -> TestResult {
    store.connection().execute(Statement::from_sql_and_values(
        DbBackend::Sqlite,
        "INSERT INTO scale_set_state (fleet_key,scale_set_id,name,runner_group,fingerprint,state,attempt_id,created_at,updated_at)
         VALUES(?,?,'runner-set','Default','sha256:identity',?,'attempt',1,2)",
        [key.into(), id.into(), state.into()],
    )).await?;
    Ok(())
}

async fn scalar(store: &Store, sql: &str) -> TestResult<i64> {
    Ok(store
        .connection()
        .query_one(Statement::from_string(DbBackend::Sqlite, sql))
        .await?
        .ok_or("missing scalar")?
        .try_get("", "value")?)
}
