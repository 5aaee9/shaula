use super::Hub;
use crate::entities::auth::fleet_auth_contexts;
use crate::entities::diagnostics::diagnostic_snapshots as snapshots;
use crate::entities::fleet::fleets;
use crate::entities::lifecycle::{fleet_sessions, runner_generations, runner_operations};
use crate::query_ext::{col, json_extract, probe};
use crate::store::{StoreError, StoreResult};
use sea_orm::{
    sea_query::{Expr, OnConflict, SimpleExpr},
    ActiveValue::Set,
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter, QuerySelect,
    TransactionTrait,
};
use shaula_core::diagnostics::*;

/// Snapshots older than this are evicted.
const RETENTION_MS: i64 = 7 * 86_400_000;

pub(crate) fn subject_key(guard: &Guard) -> String {
    // JSON framing avoids collisions between names containing separators.
    serde_json::json!([guard.generation_id, guard.fleet_key, guard.incarnation]).to_string()
}

pub(crate) async fn guard_valid(db: &impl ConnectionTrait, guard: &Guard) -> StoreResult<bool> {
    let Some(fleet) = fleets::Entity::find_by_id(guard.fleet_key.as_str())
        .one(db)
        .await?
    else {
        return Ok(false);
    };
    if fleet.incarnation != guard.incarnation
        || fleet.desired_revision != guard.revision
        || fleet.mutation_fence != guard.mutation_fence
        || fleet.tombstone
    {
        return Ok(false);
    }
    if let Some(expected) = guard.session_epoch {
        let epoch = fleet_sessions::Entity::find_by_id(guard.fleet_key.as_str())
            .select_only()
            .column(fleet_sessions::Column::Epoch)
            .into_tuple::<i64>()
            .one(db)
            .await?;
        if epoch != expected {
            return Ok(false);
        }
    }
    if let Some((key, revision)) = &guard.auth {
        let Some(context) = fleet_auth_contexts::Entity::find_by_id(guard.fleet_key.as_str())
            .one(db)
            .await?
        else {
            return Ok(false);
        };
        if context.observed_profile_key.as_ref() != Some(key)
            || context.observed_revision != Some(*revision)
        {
            return Ok(false);
        }
    }
    if let Some(id) = &guard.generation_id {
        let Some(generation) = runner_generations::Entity::find_by_id(id.as_str())
            .one(db)
            .await?
        else {
            return Ok(false);
        };
        if generation.fleet_key != guard.fleet_key
            || guard
                .generation_revision
                .is_some_and(|revision| generation.fleet_revision != revision)
            || guard
                .generation_updated_at
                .is_some_and(|v| generation.updated_at != v)
        {
            return Ok(false);
        }
        if let Some(pins) = &guard.generation_pins {
            if generation.template_profile_key != pins.profile
                || generation.template_revision != pins.revision
                || generation.template_artifact_digest != pins.artifact
                || generation.attestation_id != pins.attestation
                || generation.inputs_digest != pins.inputs
            {
                return Ok(false);
            }
        }
        if let Some(attempt) = &guard.attempt_id {
            let operation = runner_operations::Entity::find_by_id(attempt.as_str())
                .filter(runner_operations::Column::GenerationId.eq(id.as_str()))
                .select_only()
                .column(runner_operations::Column::Id)
                .into_tuple::<String>()
                .one(db)
                .await?;
            if operation.is_none() {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

pub(super) async fn write(
    db: &DatabaseConnection,
    hub: &Hub,
    o: &DecisionObservation,
) -> StoreResult<()> {
    let tx = db.begin().await?;
    // Reserve SQLite's writer before reading guards; no authority can change
    // between this read and the optional projection commit.
    tx.execute_unprepared("UPDATE diagnostic_snapshots SET sequence=sequence WHERE 0")
        .await?;
    if !hub.current(o) || !guard_valid(&tx, &o.guard).await? {
        return Ok(());
    }
    let key = subject_key(&o.guard);
    let lane = format!("{:?}", o.lane);
    let question = format!("{:?}", o.question.question);
    let mut next = o.clone();
    let old = snapshots::Entity::find_by_id((key.clone(), lane.clone(), question.clone()))
        .select_only()
        .column(snapshots::Column::Payload)
        .into_tuple::<String>()
        .one(&tx)
        .await?;
    if let Some(old) = old {
        if let Ok(previous) = serde_json::from_str::<DecisionObservation>(&old) {
            if previous.epoch == o.epoch
                && previous.sequence < o.sequence
                && o.observed_at < previous.observed_at
            {
                next.question.outcome = Outcome::Unknown;
                next.question.primary_reason_id = None;
                next.question.basis.freshness = Freshness::Unknown;
                for evidence in &mut next.question.evidence {
                    evidence.freshness = Freshness::Unknown;
                }
                for stage in &mut next.question.stages {
                    stage.evaluation = Evaluation::Unknown;
                }
                next.question.reason(
                    Code::ObservationClockInvalid,
                    next.question
                        .question
                        .stages()
                        .first()
                        .copied()
                        .unwrap_or(StageId::Authority),
                    Effect::Informational,
                    EvidenceKind::ControllerObservation,
                );
            }
            if previous.epoch == o.epoch
                && previous.guard == o.guard
                && previous.sequence.checked_add(1) == Some(o.sequence)
                && previous.observed_at <= o.observed_at
                && o.observed_at - previous.observed_at < 30_000
            {
                for reason in &mut next.question.reasons {
                    if let Some(before) = previous.question.reasons.iter().find(|r| {
                        r.code == reason.code
                            && r.stage == reason.stage
                            && r.parameters == reason.parameters
                    }) {
                        reason.first_observed_at = before.first_observed_at.clone();
                    }
                }
            }
        }
    }
    let payload = serde_json::to_string(&next)
        .map_err(|_| StoreError::Corrupt("diagnostic encoding failed".into()))?;
    if payload.len() > 65_536 {
        return Err(StoreError::Corrupt("diagnostic budget exceeded".into()));
    }
    snapshots::Entity::insert(snapshots::ActiveModel {
        subject_key: Set(key),
        lane: Set(lane),
        question: Set(question),
        epoch: Set(o.epoch.clone()),
        sequence: Set(o.sequence as i64),
        observed_at: Set(o.observed_at),
        payload: Set(payload),
    })
    .on_conflict(
        OnConflict::columns([
            snapshots::Column::SubjectKey,
            snapshots::Column::Lane,
            snapshots::Column::Question,
        ])
        .update_columns([
            snapshots::Column::Epoch,
            snapshots::Column::Sequence,
            snapshots::Column::ObservedAt,
            snapshots::Column::Payload,
        ])
        .to_owned(),
    )
    .exec_without_returning(&tx)
    .await?;
    // Only the disposable table is collected. Older live observations may also
    // be evicted; all original domain retention remains unchanged.
    snapshots::Entity::delete_many()
        .filter(snapshots::Column::ObservedAt.lt(o.observed_at.saturating_sub(RETENTION_MS)))
        .exec(&tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// `json_extract(diagnostic_snapshots.payload, path)`.
fn payload(path: &'static str) -> SimpleExpr {
    json_extract((snapshots::Entity, snapshots::Column::Payload), path)
}

/// Optional retention never deletes from the authoritative ledger.
pub(crate) async fn collect(db: &DatabaseConnection, now: i64) -> StoreResult<()> {
    let generation = || payload("$.guard.generation_id");
    let generation_gone = probe(runner_generations::Entity)
        .and_where(col(runner_generations::Column::Id).eq(generation()))
        .to_owned();
    let fleet_gone = probe(fleets::Entity)
        .and_where(col(fleets::Column::Key).eq(payload("$.guard.fleet_key")))
        .and_where(col(fleets::Column::Incarnation).eq(payload("$.guard.incarnation")))
        .to_owned();
    snapshots::Entity::delete_many()
        .filter(
            col(snapshots::Column::ObservedAt)
                .lt(now.saturating_sub(RETENTION_MS))
                .or(Expr::expr(generation())
                    .is_not_null()
                    .and(Expr::exists(generation_gone).not()))
                .or(Expr::expr(generation())
                    .is_null()
                    .and(Expr::exists(fleet_gone).not())),
        )
        .exec(db)
        .await?;
    Ok(())
}
