//! Execution references, not historical rows, prevent logical retirement.
//! Retirement calls this under its writer; status reads are observations only.

use crate::auth_dependents_repo::latest_at_or_below_head;
use crate::entities::{
    auth::fleet_session_auth as session_auth,
    fleet::{
        fleet_auth_handoffs as handoffs, fleet_changes, fleet_revision_pool_members,
        fleet_revisions as revisions, fleets,
    },
    http_state::generation_http_state as http_states,
    lifecycle::{
        fleet_sessions, runner_generations as generations, runner_operations as operations,
    },
    listener::{listener_acquisitions as acquisitions, listener_messages as messages},
    template::profile_changes,
    template_pool::{template_pool_members as pool_members, template_pools as pools},
};
use crate::query_ext::{col, json_extract, probe, same, scalar};
use crate::store::{StoreError, StoreResult};
use sea_orm::{
    sea_query::{Alias, Expr, Query, SelectStatement, SimpleExpr},
    ColumnTrait, ConnectionTrait,
};

const OPEN_OPERATIONS: [&str; 6] = [
    "Pending",
    "Starting",
    "ApplyStarting",
    "BootstrapStarting",
    "Running",
    "Blocked",
];
const OPEN_CHANGES: [&str; 3] = ["Pending", "Running", "Blocked"];
const OPEN_ACQUISITIONS: [&str; 3] = ["Pending", "AcquireStarting", "Uncertain"];

/// A lease that has an owner and has not expired (no expiry never expires).
fn leased(owner: impl ColumnTrait, expires: impl ColumnTrait + Copy, now: i64) -> SimpleExpr {
    col(owner)
        .is_not_null()
        .and(col(expires).is_null().or(col(expires).gt(now)))
}

/// A terminal Generation is not by itself proof of worker quiescence.
/// Keep open effects, worker leases and HTTP-state capabilities/locks.
fn live_generation(now: i64) -> SimpleExpr {
    let effect = probe(operations::Entity)
        .and_where(same(
            operations::Column::GenerationId,
            generations::Column::Id,
        ))
        .and_where(
            col(operations::Column::State)
                .is_in(OPEN_OPERATIONS)
                .or(leased(
                    operations::Column::LeaseOwner,
                    operations::Column::LeaseExpiresAt,
                    now,
                )),
        )
        .to_owned();
    let http = probe(http_states::Entity)
        .and_where(same(
            http_states::Column::GenerationId,
            generations::Column::Id,
        ))
        .and_where(
            col(http_states::Column::Revoked)
                .eq(false)
                .or(col(http_states::Column::LockId).is_not_null()),
        )
        .to_owned();
    col(generations::Column::State)
        .ne("Destroyed")
        .or(Expr::exists(effect))
        .or(Expr::exists(http))
}

fn live_generations(now: i64) -> SelectStatement {
    probe(generations::Entity)
        .and_where(live_generation(now))
        .to_owned()
}

/// Revisions a live Fleet executes now, or that an open change still targets.
fn live_revisions(now: i64) -> SelectStatement {
    let change = probe(fleet_changes::Entity)
        .and_where(same(
            fleet_changes::Column::FleetKey,
            revisions::Column::FleetKey,
        ))
        .and_where(same(
            fleet_changes::Column::Revision,
            revisions::Column::Revision,
        ))
        .and_where(
            col(fleet_changes::Column::State)
                .is_in(OPEN_CHANGES)
                .or(leased(
                    fleet_changes::Column::LeaseOwner,
                    fleet_changes::Column::LeaseExpiresAt,
                    now,
                )),
        )
        .to_owned();
    probe(revisions::Entity)
        .inner_join(
            fleets::Entity,
            same(fleets::Column::Key, revisions::Column::FleetKey),
        )
        .and_where(
            col(fleets::Column::Tombstone)
                .eq(false)
                .and(col(revisions::Column::Revision).eq(scalar(latest_at_or_below_head())))
                .or(Expr::exists(change)),
        )
        .to_owned()
}

fn template_refs(key: &str, now: i64) -> Vec<SelectStatement> {
    vec![
        live_generations(now)
            .and_where(col(generations::Column::TemplateProfileKey).eq(key))
            .to_owned(),
        live_revisions(now)
            .and_where(col(revisions::Column::TemplateProfileKey).eq(key))
            .to_owned(),
        live_revisions(now)
            .inner_join(
                fleet_revision_pool_members::Entity,
                same(
                    fleet_revision_pool_members::Column::FleetKey,
                    revisions::Column::FleetKey,
                )
                .and(same(
                    fleet_revision_pool_members::Column::FleetRevision,
                    revisions::Column::Revision,
                )),
            )
            .and_where(col(fleet_revision_pool_members::Column::TemplateProfileKey).eq(key))
            .to_owned(),
        live_revisions(now)
            .inner_join(
                pool_members::Entity,
                same(
                    pool_members::Column::PoolKey,
                    revisions::Column::TemplatePoolRef,
                )
                .and(same(
                    pool_members::Column::PoolRevision,
                    revisions::Column::TemplatePoolRevision,
                )),
            )
            .and_where(col(pool_members::Column::TemplateProfileKey).eq(key))
            .to_owned(),
        probe(pool_members::Entity)
            .inner_join(
                pools::Entity,
                same(pools::Column::Key, pool_members::Column::PoolKey).and(same(
                    pools::Column::DesiredRevision,
                    pool_members::Column::PoolRevision,
                )),
            )
            .and_where(col(pools::Column::Tombstone).eq(false))
            .and_where(col(pool_members::Column::TemplateProfileKey).eq(key))
            .to_owned(),
    ]
}

fn auth_refs(key: &str, now: i64) -> Vec<SelectStatement> {
    let provenance = (operations::Entity, operations::Column::ProvenanceJson);
    let jit = col(operations::Column::Kind)
        .eq("JitStarting")
        .and(json_extract(provenance, "$.context.auth_profile_key").eq(key));
    let forgejo = col(operations::Column::Kind)
        .eq("ForgejoRegistration")
        .and(json_extract(provenance, "$.auth[0]").eq(key));
    vec![
        live_revisions(now)
            .and_where(col(revisions::Column::AuthDesiredProfileKey).eq(key))
            .to_owned(),
        live_generations(now)
            .inner_join(
                revisions::Entity,
                same(revisions::Column::FleetKey, generations::Column::FleetKey).and(same(
                    revisions::Column::Revision,
                    generations::Column::FleetRevision,
                )),
            )
            .and_where(col(revisions::Column::AuthDesiredProfileKey).eq(key))
            .to_owned(),
        live_generations(now)
            .inner_join(
                operations::Entity,
                same(operations::Column::GenerationId, generations::Column::Id),
            )
            .and_where(jit.or(forgejo))
            .to_owned(),
        probe(handoffs::Entity)
            .inner_join(
                fleets::Entity,
                same(fleets::Column::Key, handoffs::Column::FleetKey),
            )
            .and_where(
                col(handoffs::Column::DesiredProfileKey)
                    .eq(key)
                    .or(col(handoffs::Column::ObservedProfileKey).eq(key)),
            )
            .and_where(col(fleets::Column::Tombstone).eq(false).or(leased(
                handoffs::Column::LeaseOwner,
                handoffs::Column::LeaseExpiresAt,
                now,
            )))
            .to_owned(),
        probe(session_auth::Entity)
            .inner_join(
                fleet_sessions::Entity,
                same(
                    fleet_sessions::Column::FleetKey,
                    session_auth::Column::FleetKey,
                ),
            )
            .and_where(col(session_auth::Column::ProfileKey).eq(key))
            .to_owned(),
        probe(messages::Entity)
            .inner_join(
                acquisitions::Entity,
                same(acquisitions::Column::FleetKey, messages::Column::FleetKey)
                    .and(same(acquisitions::Column::Epoch, messages::Column::Epoch))
                    .and(same(
                        acquisitions::Column::MessageId,
                        messages::Column::MessageId,
                    )),
            )
            .and_where(col(messages::Column::ProfileKey).eq(key))
            .and_where(col(acquisitions::Column::State).is_in(OPEN_ACQUISITIONS))
            .to_owned(),
    ]
}

pub(super) async fn in_use(
    tx: &impl ConnectionTrait,
    key: &str,
    template: bool,
    now: i64,
) -> StoreResult<bool> {
    let (kind, refs) = if template {
        ("template_profile", template_refs(key, now))
    } else {
        ("github_auth_profile", auth_refs(key, now))
    };
    let changing = probe(profile_changes::Entity)
        .and_where(col(profile_changes::Column::ResourceKind).eq(kind))
        .and_where(col(profile_changes::Column::ProfileKey).eq(key))
        .and_where(leased(
            profile_changes::Column::LeaseOwner,
            profile_changes::Column::LeaseExpiresAt,
            now,
        ))
        .to_owned();
    // One statement keeps every reference read in the same snapshot.
    let any = refs
        .into_iter()
        .fold(Expr::exists(changing), |any, r| any.or(Expr::exists(r)));
    let verdict = Query::select()
        .expr_as(any, Alias::new("in_use"))
        .to_owned();
    let row = tx
        .query_one(tx.get_database_backend().build(&verdict))
        .await?
        .ok_or_else(|| StoreError::Corrupt("missing retirement reference verdict".into()))?;
    Ok(row.try_get("", "in_use")?)
}
