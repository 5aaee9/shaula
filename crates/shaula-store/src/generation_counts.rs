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

use sea_orm::{ConnectionTrait, DbBackend, Statement, Value};
use shaula_core::lifecycle::GenerationState;
use shaula_core::template_pool::FleetPoolRef;

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
fn released() -> Value {
    GenerationState::Destroyed.as_str_repr().into()
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
    let placeholders = vec!["?"; effective.len()].join(",");
    let sql = format!(
        "SELECT COALESCE(SUM(state!=?),0) AS occupancy,
                COALESCE(SUM(state IN ({placeholders})),0) AS effective,
                COALESCE(SUM(state=?),0) AS quarantined
         FROM runner_generations WHERE fleet_key=?"
    );
    let mut values = vec![released()];
    values.extend(effective.into_iter().map(Value::from));
    values.push(GenerationState::Quarantined.as_str_repr().into());
    values.push(fleet_key.into());
    let row = db
        .query_one(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            sql,
            values,
        ))
        .await?
        .ok_or_else(|| StoreError::Corrupt("generation count aggregate returned no row".into()))?;
    Ok(GenerationCounts {
        effective: row.try_get("", "effective")?,
        occupancy: row.try_get("", "occupancy")?,
        quarantined: row.try_get("", "quarantined")?,
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
    let statement = match pool_scope {
        Some((pool_key, _)) => Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "SELECT g.pool_member_key AS member, COUNT(*) AS used
             FROM fleet_revisions r JOIN runner_generations g
               ON g.fleet_key=r.fleet_key AND g.fleet_revision=r.revision
             WHERE r.template_pool_ref=?
               AND g.state!=? AND g.pool_member_key IS NOT NULL
             GROUP BY g.pool_member_key",
            [pool_key.as_str().into(), released()],
        ),
        None => Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "SELECT pool_member_key AS member, COUNT(*) AS used FROM runner_generations
             WHERE fleet_key=? AND state!=? AND pool_member_key IS NOT NULL
             GROUP BY pool_member_key",
            [fleet_key.into(), released()],
        ),
    };
    db.query_all(statement)
        .await?
        .iter()
        .map(|row| Ok((row.try_get("", "member")?, row.try_get("", "used")?)))
        .collect()
}
