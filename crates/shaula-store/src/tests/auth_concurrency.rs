//! Regression for SQLite read-to-writer upgrades: a losing Auth replacement
//! must wait for the writer then return a domain conflict, never StorageUnavailable.

use std::time::Duration;

use shaula_core::auth::AuthRevisionInsert;

use crate::{Store, StoreError};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

fn revision(revision: i64) -> AuthRevisionInsert {
    AuthRevisionInsert {
        key: "auth".into(),
        incarnation: "incarnation".into(),
        revision,
        kind: "github_app".into(),
        app_id: Some("4863460".into()),
        schema_version: 2,
        policy_json: Some(super::auth_fixture::POLICY.into()),
    }
}

#[tokio::test]
async fn auth_replacement_waits_for_writer_before_reading_head() -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = Store::open(&dir.path().join("auth.db")).await?;
    store.migrate().await?;
    let seed = store.begin().await?;
    store
        .auth_commit_revision(&seed, revision(1), b"credential-one", 1)
        .await?;
    seed.commit().await?;

    let first = store.begin().await?;
    store
        .auth_commit_revision(&first, revision(2), b"credential-two", 2)
        .await?;
    // Hold the first replacement uncommitted. A deferred SELECT by the second
    // transaction can read r1 but cannot subsequently upgrade to this writer.
    let second = store.begin().await?;
    let competing = store.auth_commit_revision(&second, revision(2), b"credential-three", 2);
    tokio::pin!(competing);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut competing)
            .await
            .is_err(),
        "a competing replacement must wait, not fail a read-to-writer upgrade"
    );
    first.commit().await?;
    assert!(matches!(competing.await, Err(StoreError::Conflict { .. })));
    assert_eq!(
        store
            .auth_profile_get("auth")
            .await?
            .ok_or("missing head")?
            .desired_revision,
        2
    );
    Ok(())
}
