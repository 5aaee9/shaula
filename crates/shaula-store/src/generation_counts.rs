//! The single source of Generation counting rules for every fleet- and
//! pool-scoped occupancy read in the store. Counts are SQL aggregates
//! over `runner_generations`; rows (and their result payloads) are never
//! loaded just to be counted.
//!
//! Occupancy follows [`GenerationState::counts_occupancy`]: every row
//! whose state is not `Destroyed`. An unknown state string is corrupt
//! ledger data and counts as occupied (fail closed,
//! [`GenerationState::from_str_repr`]). Effective capacity follows
//! [`GenerationState::counts_effective`]; unknown strings never count.

use std::collections::HashMap;

use sea_orm::{
    sea_query::{Expr, Func, SimpleExpr},
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QuerySelect,
};
use shaula_core::lifecycle::GenerationState;
use shaula_core::template_pool::FleetPoolRef;

use crate::entities::{
    fleet::fleet_revisions as revisions, lifecycle::runner_generations as generations,
};
use crate::query_ext::{col, same, JoinOn};
use crate::store::{StoreError, StoreResult};

/// Every [`GenerationState`] variant, in declaration order. Core has no
/// variant iterator, so `state_index` matches exhaustively: a new variant
/// fails to compile here until it is given an index, and the const check
/// below ties each index to its slot in this list.
const ALL_STATES: [GenerationState; 11] = [
    GenerationState::CreatePending,
    GenerationState::Creating,
    GenerationState::WaitingOnline,
    GenerationState::Idle,
    GenerationState::Busy,
    GenerationState::Retiring,
    GenerationState::DestroyPending,
    GenerationState::Destroying,
    GenerationState::CleanupRequired,
    GenerationState::Quarantined,
    GenerationState::Destroyed,
];

const fn state_index(state: GenerationState) -> usize {
    match state {
        GenerationState::CreatePending => 0,
        GenerationState::Creating => 1,
        GenerationState::WaitingOnline => 2,
        GenerationState::Idle => 3,
        GenerationState::Busy => 4,
        GenerationState::Retiring => 5,
        GenerationState::DestroyPending => 6,
        GenerationState::Destroying => 7,
        GenerationState::CleanupRequired => 8,
        GenerationState::Quarantined => 9,
        GenerationState::Destroyed => 10,
    }
}

const _: () = {
    let mut slot = 0;
    while slot < ALL_STATES.len() {
        assert!(state_index(ALL_STATES[slot]) == slot);
        slot += 1;
    }
};

/// The only state that releases occupancy.
fn released() -> &'static str {
    GenerationState::Destroyed.as_str_repr()
}

/// `COALESCE(SUM(condition),0)`: how many rows satisfy `condition`.
fn tally(condition: SimpleExpr) -> SimpleExpr {
    Func::coalesce([Func::sum(condition).into(), Expr::val(0).into()]).into()
}

/// Fleet-scoped Generation counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct GenerationCounts {
    pub effective: i64,
    pub occupancy: i64,
    pub quarantined: i64,
}

/// One aggregate over a fleet's generations (covered by
/// `idx_runner_generations_fleet_state`).
pub(crate) async fn fleet_counts(
    db: &impl ConnectionTrait,
    fleet_key: &str,
) -> StoreResult<GenerationCounts> {
    let effective: Vec<&str> = ALL_STATES
        .into_iter()
        .filter(|s| s.counts_effective())
        .map(GenerationState::as_str_repr)
        .collect();
    let state = || col(generations::Column::State);
    let (occupancy, effective, quarantined) = generations::Entity::find()
        .select_only()
        .expr(tally(state().ne(released())))
        .expr(tally(state().is_in(effective)))
        .expr(tally(
            state().eq(GenerationState::Quarantined.as_str_repr()),
        ))
        .filter(generations::Column::FleetKey.eq(fleet_key))
        .into_tuple::<(i64, i64, i64)>()
        .one(db)
        .await?
        .ok_or_else(|| StoreError::Corrupt("generation count aggregate returned no row".into()))?;
    Ok(GenerationCounts {
        effective,
        occupancy,
        quarantined,
    })
}

/// Occupied generations per pool member: fleet-local for inline pools, or
/// across every fleet revision that froze ANY revision of the shared pool
/// (spec 0037 §6): a Fleet catches up to a new pool revision while older
/// Generations of the same member still run, and they still count.
/// Generations without a member key are not attributed to any member.
pub(crate) async fn member_occupancy(
    db: &impl ConnectionTrait,
    fleet_key: &str,
    pool_scope: Option<&FleetPoolRef>,
) -> StoreResult<HashMap<String, i64>> {
    let occupied = generations::Entity::find()
        .select_only()
        .column(generations::Column::PoolMemberKey)
        .expr(col(generations::Column::Id).count())
        .filter(generations::Column::State.ne(released()))
        .filter(generations::Column::PoolMemberKey.is_not_null())
        .group_by(generations::Column::PoolMemberKey);
    let scoped = match pool_scope {
        Some((pool_key, _)) => occupied
            .inner_join_on(
                revisions::Entity,
                same(generations::Column::FleetKey, revisions::Column::FleetKey).and(same(
                    generations::Column::FleetRevision,
                    revisions::Column::Revision,
                )),
            )
            .filter(revisions::Column::TemplatePoolRef.eq(pool_key.as_str())),
        None => occupied.filter(generations::Column::FleetKey.eq(fleet_key)),
    };
    Ok(scoped
        .into_tuple::<(String, i64)>()
        .all(db)
        .await?
        .into_iter()
        .collect())
}
