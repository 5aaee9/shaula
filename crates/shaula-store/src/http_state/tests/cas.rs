use shaula_core::state_backend::{StateAccess, StateBackend, StateCapability, StateError};

use super::{lock, state, Fixture, TestResult};

#[tokio::test]
async fn concurrent_locks_across_independent_pools_have_exactly_one_winner() -> TestResult {
    let f = Fixture::new().await?;
    let second = f.second_backend().await?;
    let (first, other) = tokio::join!(
        f.backend.lock(&f.access, lock("first")?),
        second.lock(&f.access, lock("second")?),
    );
    assert_eq!(usize::from(first.is_ok()) + usize::from(other.is_ok()), 1);
    assert!(matches!(
        (first, other),
        (Ok(()), Err(StateError::Locked(_))) | (Err(StateError::Locked(_)), Ok(()))
    ));
    Ok(())
}

#[tokio::test]
async fn lock_replay_requires_same_metadata_and_unlock_cannot_remove_a_new_lock() -> TestResult {
    let f = Fixture::new().await?;
    let first = lock("first")?;
    f.backend.lock(&f.access, first.clone()).await?;
    f.backend.lock(&f.access, first.clone()).await?;
    let changed =
        shaula_core::state_backend::LockInfo::parse(br#"{"ID":"first","Who":"impostor"}"#)?;
    assert!(matches!(
        f.backend.lock(&f.access, changed).await,
        Err(StateError::Locked(_))
    ));
    assert!(matches!(
        f.backend.unlock(&f.access, lock("wrong")?.id()).await,
        Err(StateError::Locked(_))
    ));
    f.backend.unlock(&f.access, first.id()).await?;
    f.backend.unlock(&f.access, first.id()).await?;
    f.backend.lock(&f.access, lock("second")?).await?;
    assert!(matches!(
        f.backend.unlock(&f.access, first.id()).await,
        Err(StateError::Locked(_))
    ));
    assert!(matches!(
        f.backend
            .write(&f.access, first.id(), state(0, "lineage", true)?)
            .await,
        Err(StateError::Locked(_))
    ));
    Ok(())
}

#[tokio::test]
async fn writes_require_current_owner_and_lock_even_for_identical_replays() -> TestResult {
    let f = Fixture::new().await?;
    let info = lock("owner")?;
    assert!(matches!(
        f.backend
            .write(&f.access, info.id(), state(0, "lineage", false)?)
            .await,
        Err(StateError::Conflict)
    ));
    f.backend.lock(&f.access, info.clone()).await?;
    let wrong = StateAccess {
        generation_id: f.access.generation_id,
        capability: StateCapability::issue(),
    };
    assert!(matches!(
        f.backend
            .write(&wrong, info.id(), state(0, "lineage", false)?)
            .await,
        Err(StateError::Unauthorized)
    ));
    assert_eq!(
        f.backend
            .write(&f.access, info.id(), state(0, "lineage", false)?)
            .await?,
        1
    );
    f.backend.unlock(&f.access, info.id()).await?;
    assert!(matches!(
        f.backend
            .write(&f.access, info.id(), state(0, "lineage", false)?)
            .await,
        Err(StateError::Conflict)
    ));
    Ok(())
}

#[tokio::test]
async fn lineage_serial_and_byte_exact_replay_are_transactional() -> TestResult {
    let f = Fixture::new().await?;
    let info = lock("owner")?;
    f.backend.lock(&f.access, info.clone()).await?;
    assert_eq!(
        f.backend
            .write(&f.access, info.id(), state(1, "lineage", true)?)
            .await?,
        1
    );
    // Commit succeeded, response was lost, daemon restarted: same bytes do not
    // increment backend revision a second time.
    let reopened = f.second_backend().await?;
    assert_eq!(
        reopened
            .write(&f.access, info.id(), state(1, "lineage", true)?)
            .await?,
        1
    );
    for document in [
        state(0, "lineage", true)?,
        state(1, "lineage", false)?,
        state(2, "other", true)?,
    ] {
        assert!(matches!(
            f.backend.write(&f.access, info.id(), document).await,
            Err(StateError::Conflict)
        ));
    }
    let snapshot = f.backend.read(&f.access).await?.ok_or("missing state")?;
    assert_eq!(snapshot.revision, 1);
    assert_eq!(
        snapshot.document.bytes(),
        state(1, "lineage", true)?.bytes()
    );
    assert_eq!(
        f.backend
            .write(&f.access, info.id(), state(7, "lineage", false)?)
            .await?,
        2
    );
    Ok(())
}

#[tokio::test]
async fn unlock_relock_and_write_cannot_split_the_ownership_check_from_commit() -> TestResult {
    let f = Fixture::new().await?;
    let other = f.second_backend().await?;
    let old = lock("old")?;
    let new = lock("new")?;
    f.backend.lock(&f.access, old.clone()).await?;
    f.backend
        .write(&f.access, old.id(), state(0, "lineage", true)?)
        .await?;
    // Hold the database writer while a stale write queues on another pool.
    let tx = f.backend.writer(&f.claim.generation_id.to_string()).await?;
    let queued_backend = other.clone();
    let access = f.access.clone();
    let old_id = old.id().clone();
    let writer = tokio::spawn(async move {
        queued_backend
            .write(&access, &old_id, state(1, "lineage", false)?)
            .await
    });
    use sea_orm::ConnectionTrait;
    tx.execute(super::sql(
        "UPDATE generation_http_state SET lock_id = ?, lock_info = ? WHERE generation_id = ?",
        vec![
            new.id().expose().into(),
            new.to_bytes()?.into(),
            f.claim.generation_id.to_string().into(),
        ],
    ))
    .await?;
    tx.commit().await?;
    assert!(matches!(writer.await?, Err(StateError::Locked(_))));
    let snapshot = other.read(&f.access).await?.ok_or("missing state")?;
    assert_eq!(snapshot.revision, 1);
    assert!(!snapshot.document.managed_empty());
    Ok(())
}

#[tokio::test]
async fn concurrent_serials_never_roll_state_back() -> TestResult {
    let f = Fixture::new().await?;
    let second = f.second_backend().await?;
    let info = lock("owner")?;
    f.backend.lock(&f.access, info.clone()).await?;
    let (low, high) = tokio::join!(
        f.backend
            .write(&f.access, info.id(), state(1, "lineage", true)?),
        second.write(&f.access, info.id(), state(2, "lineage", false)?),
    );
    high?;
    assert!(low.is_ok() || matches!(low, Err(StateError::Conflict)));
    assert_eq!(
        f.backend
            .read(&f.access)
            .await?
            .ok_or("missing state")?
            .document
            .serial(),
        2
    );
    Ok(())
}
