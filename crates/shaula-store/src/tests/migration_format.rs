//! Durable-format migration gate tests (R6, spec 0011 §7): the 1→2
//! conversion is legal only from the supported format-1 encoding. A
//! format-0 database stays REJECTED and UNCHANGED; a supported format-1
//! database upgrades with legacy refs, secrets and replay facts intact.

#![allow(clippy::unwrap_used)]

use crate::Store;
use sea_orm::{ConnectionTrait, Statement, TransactionTrait};
use shaula_store_migration as migration;
use shaula_store_migration::MigratorTrait;

const MARKER_SQL: &str = "SELECT version FROM durable_format";
const SEED_PROFILE: &str = "INSERT INTO github_auth_profiles \
     (key, incarnation, desired_revision, active_revision, observed_revision, status, \
      deletion_requested, created_at, updated_at) \
     VALUES ('legacy-app', 'inc-1', 1, 1, 1, 'Active', 0, 10, 10)";
const SEED_REVISION: &str = "INSERT INTO github_auth_profile_revisions \
     (profile_key, revision, kind, app_id, installation_id, pat_principal, allowlist_json, \
      credential_bytes, state, reason, created_at) \
     VALUES ('legacy-app', 1, 'github_app', 'Iv23legacy', 34, NULL, \
      '[{\"kind\":\"organization\",\"owner\":\"example-org\"}]', \
      X'70656D2D6279746573', 'Active', NULL, 10)";
const SEED_IDEMPOTENCY: &str = "INSERT INTO idempotency_records \
     (id, resource_kind, resource_key, idempotency_key, request_hash, response_status, \
      response_body, created_at) \
     VALUES ('idem-1', 'github_auth_profile', 'legacy-app', 'put-1', \
      'sha256:baseline', 202, '{\"changeId\":\"change-1\"}', 10)";

/// Opens a fresh database and returns the handle plus its path (kept
/// alive via mem::forget, the established test pattern).
async fn fresh() -> (Store, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("m8.db");
    std::mem::forget(tmp);
    let store = Store::open(&path).await.unwrap();
    (store, path)
}

async fn marker_of(store: &Store) -> Option<i64> {
    let row = store
        .connection()
        .query_one(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            MARKER_SQL,
        ))
        .await
        .unwrap();
    row.and_then(|r| r.try_get::<i64>("", "version").ok())
}

/// Applies only m0001..m0007 (the pre-multi-account schema).
async fn migrate_to_pre_m0008(store: &Store) {
    // This bootstraps the schema before durable_format exists, so use the
    // migration connection rather than the post-migration runtime boundary.
    let tx = store.connection().begin().await.unwrap();
    migration::Migrator::up(&tx, Some(7)).await.unwrap();
    tx.commit().await.unwrap();
}

async fn execute(store: &Store, sql: &str) {
    store.connection().execute_unprepared(sql).await.unwrap();
}

/// R6: a populated format-0 database (deliberately unsupported legacy
/// identity encodings) stays REJECTED and UNCHANGED — the migration
/// fails, the surrounding transaction rolls back the DDL, and the marker
/// remains 0.
#[tokio::test]
async fn format_zero_stays_rejected_and_unchanged() {
    let (store, _path) = fresh().await;
    migrate_to_pre_m0008(&store).await;
    execute(&store, SEED_PROFILE).await;
    execute(&store, SEED_REVISION).await;
    execute(&store, SEED_IDEMPOTENCY).await;
    // Simulate the pre-release marker for a populated database.
    execute(&store, "UPDATE durable_format SET version = 0").await;

    // The multi-account migration must FAIL (rolling back its DDL), and
    // a subsequent full migrate keeps failing until the operator
    // rebuilds the data directory.
    let outcome = migration::migrate(store.connection()).await;
    assert!(outcome.is_err(), "format 0 must not convert");
    assert_eq!(marker_of(&store).await, Some(0), "marker unchanged");
    // The rolled-back DDL left the schema untouched: no new table.
    let new_tables = table_count(&store, "github_auth_revision_bindings").await;
    assert_eq!(
        new_tables, 0,
        "bindings table must not exist after rollback"
    );
    // Legacy rows remain readable in their original schema.
    let legacy_rows = scalar(
        &store,
        "SELECT COUNT(*) AS n FROM github_auth_profile_revisions",
    )
    .await;
    assert_eq!(legacy_rows, 1);
}

/// R6: a supported format-1 database upgrades: the marker becomes 2
/// while legacy refs, credential bytes and idempotency replay facts stay
/// byte-identical; the store format gate then accepts the directory.
#[tokio::test]
async fn format_one_upgrades_preserving_legacy_rows() {
    let (store, path) = fresh().await;
    migrate_to_pre_m0008(&store).await;
    execute(&store, SEED_PROFILE).await;
    execute(&store, SEED_REVISION).await;
    execute(&store, SEED_IDEMPOTENCY).await;

    migration::migrate(store.connection()).await.unwrap();
    assert_eq!(
        marker_of(&store).await,
        Some(migration::m0008_auth_multi_account::DURABLE_FORMAT_VERSION_V2),
    );
    // Legacy rows keep their exact representation: the default
    // schema_version is 1 and every historical member survives.
    let stored: String = {
        let row = store
            .connection()
            .query_one(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT allowlist_json FROM github_auth_profile_revisions",
            ))
            .await
            .unwrap()
            .unwrap();
        row.try_get::<String>("", "allowlist_json").unwrap()
    };
    eprintln!("DEBUG stored allowlist = {stored}");
    for (sql, label) in [
        (
            "SELECT COUNT(*) AS n FROM github_auth_profile_revisions \
             WHERE profile_key='legacy-app' AND kind='github_app' \
             AND app_id='Iv23legacy' AND installation_id=34 AND state='Active'",
            "core members",
        ),
        (
            "SELECT COUNT(*) AS n FROM github_auth_profile_revisions \
             WHERE allowlist_json='[{\"kind\":\"organization\",\"owner\":\"example-org\"}]'",
            "allowlist bytes",
        ),
        (
            "SELECT COUNT(*) AS n FROM github_auth_profile_revisions \
             WHERE credential_bytes=X'70656D2D6279746573'",
            "credential bytes",
        ),
        (
            "SELECT COUNT(*) AS n FROM github_auth_profile_revisions \
             WHERE schema_version=1",
            "schema_version defaulted to legacy 1",
        ),
    ] {
        assert_eq!(scalar(&store, sql).await, 1, "{label}");
    }
    let idem = scalar(
        &store,
        "SELECT COUNT(*) AS n FROM idempotency_records \
         WHERE resource_key='legacy-app' AND request_hash='sha256:baseline' \
         AND response_body='{\"changeId\":\"change-1\"}'",
    )
    .await;
    assert_eq!(idem, 1, "replay fact preserved");
    // The durable-format gate accepts the migrated directory.
    let reopened = Store::open(&path).await.unwrap();
    reopened.migrate().await.unwrap();
}

async fn table_count(store: &Store, name: &str) -> i64 {
    scalar(
        store,
        &format!("SELECT COUNT(*) AS n FROM sqlite_master WHERE type='table' AND name='{name}'"),
    )
    .await
}

async fn scalar(store: &Store, sql: &str) -> i64 {
    let row = store
        .connection()
        .query_one(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            sql.to_string(),
        ))
        .await
        .unwrap()
        .unwrap();
    row.try_get::<i64>("", "n").unwrap()
}
