//! Captured network work cannot install or clear a newer session.

use sea_orm::ConnectionTrait;
use shaula_core::auth_context::ResolvedAuthContext;
use shaula_core::registry::SessionInstall;

use super::session_support::{install, request};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

pub(super) async fn established() -> TestResult<(crate::Store, ResolvedAuthContext)> {
    let (store, captured, context) = super::auth_execution::ready().await;
    assert_eq!(
        store
            .handoff_acknowledge(
                "fleet",
                super::auth_v2::PROFILE,
                1,
                Some(&serde_json::to_string(&context)?),
                &captured
            )
            .await?,
        shaula_core::registry::FleetContextAck::Acknowledged
    );
    assert_eq!(
        install(&store, "fleet", "original", 42, &context, 10).await?,
        1
    );
    Ok((store, context))
}

async fn replacement(
    store: &crate::Store,
    context: &ResolvedAuthContext,
) -> TestResult<SessionInstall> {
    Ok(request(store, "fleet", "replacement", 42, context).await?)
}

#[tokio::test]
async fn complete_handle_and_initial_demand_commit_together() -> TestResult {
    let (store, context) = established().await?;
    let session = store
        .session_handle("fleet")
        .await?
        .ok_or("missing active")?;
    assert_eq!(session.epoch, 1);
    assert_eq!(session.scale_set_id, 42);
    assert_eq!(session.auth_context, context);
    assert_eq!(session.handle.session_id, "original");
    assert_eq!(
        session.handle.message_queue_url,
        "https://queue.test/messages"
    );
    assert_eq!(
        session.handle.message_queue_access_token,
        "test-session-secret"
    );
    assert_eq!(session.handle.initial_statistics.total_assigned_jobs, 3);
    assert!(!format!("{session:?}").contains("test-session-secret"));
    let mut next = replacement(&store, &context).await?;
    next.handle.message_queue_access_token.clear();
    next.handle.initial_statistics.total_assigned_jobs = 999;
    assert!(store.session_install("fleet", &next, 11).await.is_err());
    assert_eq!(
        store
            .session_get("fleet")
            .await?
            .ok_or("missing row")?
            .epoch,
        1
    );
    assert_eq!(
        store
            .demand_get("fleet")
            .await?
            .ok_or("missing row")?
            .total_assigned_jobs,
        3
    );
    Ok(())
}
#[tokio::test]
async fn session_install_refuses_each_stale_fleet_authority_without_writes() -> TestResult {
    for mutation in [
        "UPDATE fleets SET incarnation='replacement' WHERE key='fleet'",
        "UPDATE fleets SET desired_revision=2 WHERE key='fleet'",
        "UPDATE fleets SET mutation_fence=2 WHERE key='fleet'",
        "UPDATE fleets SET deletion_marker=1 WHERE key='fleet'",
        "UPDATE fleets SET tombstone=1 WHERE key='fleet'",
        "UPDATE fleet_auth_handoffs SET desired_revision=2 WHERE fleet_key='fleet'",
        "UPDATE fleet_auth_contexts SET state='Blocked' WHERE fleet_key='fleet'",
        "UPDATE scale_set_state SET state='AccessBlocked' WHERE fleet_key='fleet'",
    ] {
        let (store, context) = established().await?;
        let next = replacement(&store, &context).await?;
        store.connection().execute_unprepared(mutation).await?;
        assert_eq!(
            store.session_install("fleet", &next, 11).await?,
            None,
            "{mutation}"
        );
        let original = store.session_get("fleet").await?.ok_or("missing row")?;
        assert_eq!(original.epoch, 1, "{mutation}");
        assert_eq!(original.session_id, "original", "{mutation}");
        assert_eq!(
            store
                .demand_get("fleet")
                .await?
                .ok_or("missing row")?
                .total_assigned_jobs,
            3
        );
    }
    Ok(())
}
#[tokio::test]
async fn late_session_creation_and_release_cannot_overwrite_the_new_epoch() -> TestResult {
    let (store, context) = established().await?;
    let old_attempt = replacement(&store, &context).await?;
    let mut first = old_attempt.clone();
    first.handle.session_id = "winner".into();
    assert_eq!(store.session_install("fleet", &first, 11).await?, Some(2));
    assert_eq!(
        store.session_install("fleet", &old_attempt, 12).await?,
        None
    );
    assert!(!store.session_clear("fleet", 1, 12).await?);
    assert_eq!(
        store
            .session_handle("fleet")
            .await?
            .ok_or("missing active")?
            .handle
            .session_id,
        "winner"
    );
    assert!(store.session_clear("fleet", 2, 13).await?);
    assert!(store.session_handle("fleet").await?.is_none());
    assert_eq!(
        store
            .session_get("fleet")
            .await?
            .ok_or("missing epoch retained")?
            .epoch,
        3
    );
    let after_clear = replacement(&store, &context).await?;
    assert_eq!(
        store.session_install("fleet", &after_clear, 14).await?,
        Some(4)
    );
    assert!(!store.session_clear("fleet", 2, 15).await?);
    Ok(())
}
#[tokio::test]
async fn session_guard_checks_exact_authority_and_epoch() -> TestResult {
    let (store, context) = established().await?;
    let next = replacement(&store, &context).await?;
    let tx = store.begin().await?;
    assert!(
        store
            .session_guard_tx(&tx, "fleet", &next.guard, &context, 1)
            .await?
    );
    assert!(
        !store
            .session_guard_tx(&tx, "fleet", &next.guard, &context, 2)
            .await?
    );
    let mut wrong = context;
    wrong.installation_id += 1;
    assert!(
        !store
            .session_guard_tx(&tx, "fleet", &next.guard, &wrong, 1)
            .await?
    );
    tx.rollback().await?;
    Ok(())
}

#[tokio::test]
async fn current_decommission_can_close_a_predecessor_but_stale_heads_and_tombstones_cannot(
) -> TestResult {
    let (store, context) = established().await?;
    let mut guard = replacement(&store, &context).await?.guard;
    assert!(store.session_close_authorize("fleet", &guard).await?);
    store.connection().execute_unprepared(
        "UPDATE fleets SET desired_revision=2,mutation_fence=2,deletion_marker=1 WHERE key='fleet'"
    ).await?;
    assert!(!store.session_close_authorize("fleet", &guard).await?);
    guard.desired_revision = 2;
    guard.mutation_fence = 2;
    assert!(store.session_close_authorize("fleet", &guard).await?);
    let tx = store.begin().await?;
    assert!(
        !store
            .runtime_auth_guard_tx(&tx, "fleet", &guard, &context)
            .await?
    );
    tx.rollback().await?;
    let mut stale_incarnation = guard.clone();
    stale_incarnation.incarnation = "another".into();
    assert!(
        !store
            .session_close_authorize("fleet", &stale_incarnation)
            .await?
    );
    store
        .connection()
        .execute_unprepared("UPDATE fleets SET tombstone=1 WHERE key='fleet'")
        .await?;
    assert!(!store.session_close_authorize("fleet", &guard).await?);
    Ok(())
}
