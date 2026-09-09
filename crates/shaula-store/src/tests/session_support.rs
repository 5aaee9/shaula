//! Explicit session fixtures use the same captured authority as production.

use shaula_core::auth_context::ResolvedAuthContext;
use shaula_core::ports::{SessionHandle, StatisticsSnapshot};
use shaula_core::registry::{FleetRuntimeGuard, SessionInstall};

use crate::{Store, StoreError, StoreResult};

pub(super) async fn install(
    store: &Store,
    fleet: &str,
    id: &str,
    scale_set_id: i64,
    context: &ResolvedAuthContext,
    now: i64,
) -> StoreResult<i64> {
    let install = request(store, fleet, id, scale_set_id, context).await?;
    store
        .scale_set_upsert(shaula_core::registry::ScaleSetRow {
            fleet_key: fleet.into(),
            scale_set_id: Some(scale_set_id),
            name: "fixture".into(),
            runner_group: "Default".into(),
            fingerprint: "fixture".into(),
            state: "Adopted".into(),
            attempt_id: None,
            now,
        })
        .await?;
    store
        .session_install(fleet, &install, now)
        .await?
        .ok_or_else(|| StoreError::Conflict {
            resource: "session authority changed".into(),
        })
}

pub(super) async fn request(
    store: &Store,
    fleet: &str,
    id: &str,
    scale_set_id: i64,
    context: &ResolvedAuthContext,
) -> StoreResult<SessionInstall> {
    let guard = match store.fleet_get(fleet).await? {
        Some(head) => FleetRuntimeGuard {
            incarnation: head.incarnation,
            desired_revision: head.desired_revision,
            mutation_fence: head.mutation_fence,
        },
        None => FleetRuntimeGuard {
            incarnation: "missing".into(),
            desired_revision: 1,
            mutation_fence: 1,
        },
    };
    Ok(SessionInstall {
        guard,
        expected_epoch: store.session_get(fleet).await?.map(|s| s.epoch),
        auth_context: context.clone(),
        scale_set_id,
        handle: SessionHandle {
            session_id: id.into(),
            message_queue_url: "https://queue.test/messages".into(),
            message_queue_access_token: "test-session-secret".into(),
            initial_statistics: StatisticsSnapshot {
                total_assigned_jobs: 3,
                ..Default::default()
            },
        },
    })
}
