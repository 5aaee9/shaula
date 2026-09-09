use sea_orm::ConnectionTrait;
use shaula_core::state_backend::{StateAccess, StateBackend, StateCapability, StateError};
use uuid::Uuid;

use super::{lock, record, sql, state, Fixture, TestResult};

#[tokio::test]
async fn fresh_state_is_distinct_from_unknown_legacy_and_corrupt_state() -> TestResult {
    let f = Fixture::new().await?;
    assert!(f.backend.read(&f.access).await?.is_none());
    let unknown = StateAccess {
        generation_id: Uuid::new_v4(),
        capability: f.access.capability.clone(),
    };
    assert!(matches!(
        f.backend.read(&unknown).await,
        Err(StateError::Unauthorized)
    ));
    // A local-state row must not acquire a fresh HTTP backend by re-admission.
    f.store
        .generation_insert(record(unknown.generation_id))
        .await?;
    assert!(matches!(
        f.backend
            .insert_generation(record(unknown.generation_id), Uuid::new_v4())
            .await,
        Err(StateError::Conflict)
    ));
    assert!(matches!(
        f.backend.read(&unknown).await,
        Err(StateError::Unauthorized)
    ));
    f.store.migrate().await?;
    assert!(matches!(
        f.backend.read(&unknown).await,
        Err(StateError::Unauthorized)
    ));
    let info = lock("owner")?;
    f.backend.lock(&f.access, info.clone()).await?;
    f.backend
        .write(&f.access, info.id(), state(0, "lineage", true)?)
        .await?;
    f.corrupt("state_bytes = x'7b7d'").await?; // {} is corrupt, never fresh/empty.
    assert!(matches!(
        f.backend.read(&f.access).await,
        Err(StateError::Unavailable)
    ));
    assert!(matches!(
        f.backend
            .write(&f.access, info.id(), state(1, "lineage", false)?)
            .await,
        Err(StateError::Unavailable)
    ));
    Ok(())
}

#[tokio::test]
async fn missing_after_create_and_lost_backend_row_never_return_fresh_state() -> TestResult {
    let f = Fixture::new().await?;
    f.corrupt("create_started = 1").await?;
    assert!(matches!(
        f.backend.read(&f.access).await,
        Err(StateError::Unavailable)
    ));
    assert!(matches!(
        f.backend.lock(&f.access, lock("owner")?).await,
        Err(StateError::Unavailable)
    ));
    f.store
        .connection()
        .execute(sql(
            "DELETE FROM generation_http_state WHERE generation_id = ?",
            vec![f.claim.generation_id.to_string().into()],
        ))
        .await?;
    assert!(matches!(
        f.backend.read(&f.access).await,
        Err(StateError::Unauthorized)
    ));
    assert!(matches!(
        f.backend
            .insert_generation(record(f.claim.generation_id), Uuid::new_v4())
            .await,
        Err(StateError::Conflict)
    ));
    Ok(())
}

#[tokio::test]
async fn admission_is_atomic_and_fenced_by_current_fleet_head() -> TestResult {
    let f = Fixture::new().await?;
    let id = Uuid::new_v4();
    let mut stale = record(id);
    stale.fleet_revision = 2;
    assert!(matches!(
        f.backend.insert_generation(stale, Uuid::new_v4()).await,
        Err(StateError::Conflict)
    ));
    assert!(f.store.generation_get(&id.to_string()).await?.is_none());
    f.store
        .connection()
        .execute_unprepared("UPDATE fleets SET deletion_marker = 1")
        .await?;
    assert!(matches!(
        f.backend
            .insert_generation(record(id), Uuid::new_v4())
            .await,
        Err(StateError::Conflict)
    ));
    assert!(f.store.generation_get(&id.to_string()).await?.is_none());
    Ok(())
}

#[tokio::test]
async fn revocation_preserves_locks_and_rechecks_authority_after_authentication() -> TestResult {
    let f = Fixture::new().await?;
    let info = lock("orphan")?;
    f.backend.lock(&f.access, info.clone()).await?;
    f.backend.authenticate(&f.access).await?;
    f.backend.revoke(&f.claim).await?;
    assert!(matches!(
        f.backend
            .write(&f.access, info.id(), state(0, "lineage", true)?)
            .await,
        Err(StateError::Unauthorized)
    ));
    assert!(matches!(
        f.backend.unlock(&f.access, info.id()).await,
        Err(StateError::Unauthorized)
    ));
    let row =
        super::super::row::Row::load(f.store.connection(), &f.claim.generation_id.to_string())
            .await?;
    assert_eq!(row.worker_epoch, 1);
    assert_eq!(
        row.lock(f.store.connection()).await?.ok_or("lost lock")?,
        info
    );
    // Revocation is idempotent, but not authority to replace another Claim.
    f.backend.revoke(&f.claim).await?;
    let mut stale = f.claim.clone();
    stale.worker_epoch += 1;
    assert!(matches!(
        f.backend.revoke(&stale).await,
        Err(StateError::Conflict)
    ));
    Ok(())
}

#[tokio::test]
async fn stale_capability_cannot_write_even_if_new_worker_reuses_lock_id() -> TestResult {
    let f = Fixture::new().await?;
    let info = lock("same-id")?;
    f.backend.lock(&f.access, info.clone()).await?;
    let replacement = StateCapability::issue();
    // Fixture for a completed, externally proven recovery. There is deliberately
    // no production takeover API until the Executor can supply that proof.
    f.store.connection().execute(sql(
        "UPDATE generation_http_state SET worker_epoch = 2, worker_attempt = ?, capability_hash = ?
         WHERE generation_id = ?",
        vec![Uuid::new_v4().to_string().into(), replacement.verifier().into(), f.claim.generation_id.to_string().into()],
    )).await?;
    assert!(matches!(
        f.backend
            .write(&f.access, info.id(), state(0, "lineage", true)?)
            .await,
        Err(StateError::Unauthorized)
    ));
    assert!(matches!(
        f.backend.unlock(&f.access, info.id()).await,
        Err(StateError::Unauthorized)
    ));
    assert!(matches!(
        f.backend.revoke(&f.claim).await,
        Err(StateError::Conflict)
    ));
    Ok(())
}

#[tokio::test]
async fn seal_requires_exact_empty_version_and_no_lock_but_never_releases_occupancy() -> TestResult
{
    let f = Fixture::new().await?;
    let info = lock("owner")?;
    assert!(f.backend.note_create_starting(&f.claim).await.is_err());
    f.backend.lock(&f.access, info.clone()).await?;
    f.backend
        .write(&f.access, info.id(), state(0, "lineage", false)?)
        .await?;
    assert!(f.backend.seal(&f.claim, 1).await.is_err());
    f.backend.note_create_starting(&f.claim).await?;
    assert!(f.backend.note_create_starting(&f.claim).await.is_err());
    f.backend
        .write(&f.access, info.id(), state(1, "lineage", true)?)
        .await?;
    f.backend.unlock(&f.access, info.id()).await?;
    assert!(f.backend.seal(&f.claim, 2).await.is_err());
    f.backend.lock(&f.access, info.clone()).await?;
    f.backend
        .write(&f.access, info.id(), state(2, "lineage", false)?)
        .await?;
    assert!(f.backend.seal(&f.claim, 3).await.is_err());
    f.backend.unlock(&f.access, info.id()).await?;
    assert!(f.backend.seal(&f.claim, 2).await.is_err());
    f.backend.seal(&f.claim, 3).await?;
    f.backend.seal(&f.claim, 3).await?;
    assert!(matches!(
        f.backend.lock(&f.access, info.clone()).await,
        Err(StateError::Sealed)
    ));
    assert!(matches!(
        f.backend
            .write(&f.access, info.id(), state(3, "lineage", false)?)
            .await,
        Err(StateError::Sealed)
    ));
    let generation = f
        .store
        .generation_get(&f.claim.generation_id.to_string())
        .await?
        .ok_or("missing generation")?;
    assert_eq!(
        generation.state, "CreatePending",
        "backend sealing cannot release resources"
    );
    Ok(())
}

#[tokio::test]
async fn uncommitted_write_rolls_back_and_all_pool_connections_are_durable() -> TestResult {
    let f = Fixture::new().await?;
    let info = lock("owner")?;
    f.backend.lock(&f.access, info.clone()).await?;
    f.backend
        .write(&f.access, info.id(), state(0, "lineage", true)?)
        .await?;
    let tx = f.backend.writer(&f.claim.generation_id.to_string()).await?;
    tx.execute(sql(
        "UPDATE generation_http_state SET state_bytes = ?, serial = 1, revision = 2",
        vec![state(1, "lineage", false)?.bytes().to_vec().into()],
    ))
    .await?;
    drop(tx); // cancellation before commit must roll back on the pool connection.
    let reopened = f.second_backend().await?;
    let snapshot = reopened.read(&f.access).await?.ok_or("missing state")?;
    assert_eq!(snapshot.revision, 1);
    assert!(!snapshot.document.managed_empty());
    let mut held = Vec::new();
    for _ in 0..8 {
        // Pin every pool connection to inspect connection-local pragmas.
        // These are read-only probes, not runtime read/write transactions.
        use sea_orm::TransactionTrait;
        let tx = f.store.connection().begin().await?;
        let row = tx
            .query_one(sql("PRAGMA synchronous", vec![]))
            .await?
            .ok_or("missing pragma")?;
        assert_eq!(row.try_get::<i64>("", "synchronous")?, 2);
        let row = tx
            .query_one(sql("PRAGMA foreign_keys", vec![]))
            .await?
            .ok_or("missing pragma")?;
        assert_eq!(row.try_get::<i64>("", "foreign_keys")?, 1);
        held.push(tx);
    }
    Ok(())
}
