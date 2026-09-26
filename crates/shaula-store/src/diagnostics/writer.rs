use super::Hub;
use crate::store::{StoreError, StoreResult};
use sea_orm::{
    ConnectionTrait, DatabaseBackend::Sqlite, DatabaseConnection, Statement, TransactionTrait,
};
use shaula_core::diagnostics::*;

pub(crate) fn subject_key(guard: &Guard) -> String {
    // JSON framing avoids collisions between names containing separators.
    serde_json::json!([guard.generation_id, guard.fleet_key, guard.incarnation]).to_string()
}

pub(crate) async fn guard_valid(db: &impl ConnectionTrait, guard: &Guard) -> StoreResult<bool> {
    let row = db
        .query_one(Statement::from_sql_and_values(
            Sqlite,
            "SELECT incarnation,desired_revision,mutation_fence,tombstone FROM fleets WHERE key=?",
            vec![guard.fleet_key.clone().into()],
        ))
        .await?;
    let Some(row) = row else { return Ok(false) };
    if row.try_get::<String>("", "incarnation")? != guard.incarnation
        || row.try_get::<i64>("", "desired_revision")? != guard.revision
        || row.try_get::<i64>("", "mutation_fence")? != guard.mutation_fence
        || row.try_get::<bool>("", "tombstone")?
    {
        return Ok(false);
    }
    if let Some(expected) = guard.session_epoch {
        let row = db
            .query_one(Statement::from_sql_and_values(
                Sqlite,
                "SELECT epoch FROM fleet_sessions WHERE fleet_key=?",
                vec![guard.fleet_key.clone().into()],
            ))
            .await?;
        let epoch = row.map(|r| r.try_get::<i64>("", "epoch")).transpose()?;
        if epoch != expected {
            return Ok(false);
        }
    }
    if let Some((key, revision)) = &guard.auth {
        let row = db.query_one(Statement::from_sql_and_values(Sqlite,
            "SELECT observed_profile_key,observed_revision FROM fleet_auth_contexts WHERE fleet_key=?",
            vec![guard.fleet_key.clone().into()])).await?;
        let Some(row) = row else { return Ok(false) };
        if row
            .try_get::<Option<String>>("", "observed_profile_key")?
            .as_ref()
            != Some(key)
            || row.try_get::<Option<i64>>("", "observed_revision")? != Some(*revision)
        {
            return Ok(false);
        }
    }
    if let Some(id) = &guard.generation_id {
        let row = db
            .query_one(Statement::from_sql_and_values(
                Sqlite,
                "SELECT fleet_key,fleet_revision,updated_at,template_profile_key,template_revision,template_artifact_digest,attestation_id,inputs_digest FROM runner_generations WHERE id=?",
                vec![id.clone().into()],
            ))
            .await?;
        let Some(row) = row else { return Ok(false) };
        if row.try_get::<String>("", "fleet_key")? != guard.fleet_key
            || guard.generation_revision.is_some_and(|revision| {
                row.try_get::<i64>("", "fleet_revision").ok() != Some(revision)
            })
            || guard
                .generation_updated_at
                .is_some_and(|v| row.try_get::<i64>("", "updated_at").ok() != Some(v))
        {
            return Ok(false);
        }
        if let Some(pins) = &guard.generation_pins {
            if row.try_get::<String>("", "template_profile_key")? != pins.profile
                || row.try_get::<i64>("", "template_revision")? != pins.revision
                || row.try_get::<String>("", "template_artifact_digest")? != pins.artifact
                || row.try_get::<String>("", "attestation_id")? != pins.attestation
                || row.try_get::<String>("", "inputs_digest")? != pins.inputs
            {
                return Ok(false);
            }
        }
        if let Some(attempt) = &guard.attempt_id {
            if db
                .query_one(Statement::from_sql_and_values(
                    Sqlite,
                    "SELECT 1 FROM runner_operations WHERE generation_id=? AND id=?",
                    vec![id.clone().into(), attempt.clone().into()],
                ))
                .await?
                .is_none()
            {
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
    let old = tx.query_one(Statement::from_sql_and_values(Sqlite,
        "SELECT payload FROM diagnostic_snapshots WHERE subject_key=? AND lane=? AND question=?",
        vec![key.clone().into(), lane.clone().into(), question.clone().into()])).await?;
    if let Some(old) = old {
        if let Ok(previous) =
            serde_json::from_str::<DecisionObservation>(&old.try_get::<String>("", "payload")?)
        {
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
    tx.execute(Statement::from_sql_and_values(Sqlite,
        "INSERT INTO diagnostic_snapshots(subject_key,lane,question,epoch,sequence,observed_at,payload) VALUES(?,?,?,?,?,?,?)
         ON CONFLICT(subject_key,lane,question) DO UPDATE SET epoch=excluded.epoch,sequence=excluded.sequence,observed_at=excluded.observed_at,payload=excluded.payload",
        vec![key.into(),lane.into(),question.into(),o.epoch.clone().into(),(o.sequence as i64).into(),o.observed_at.into(),payload.into()])).await?;
    // Only the disposable table is collected. Older live observations may also
    // be evicted; all original domain retention remains unchanged.
    tx.execute(Statement::from_sql_and_values(
        Sqlite,
        "DELETE FROM diagnostic_snapshots WHERE observed_at < ?",
        vec![o.observed_at.saturating_sub(7 * 86_400_000).into()],
    ))
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Optional retention never deletes from the authoritative ledger.
pub(crate) async fn collect(db: &DatabaseConnection, now: i64) -> StoreResult<()> {
    db.execute(Statement::from_sql_and_values(Sqlite,
        "DELETE FROM diagnostic_snapshots WHERE observed_at < ? OR
         (json_extract(payload,'$.guard.generation_id') IS NOT NULL AND NOT EXISTS
          (SELECT 1 FROM runner_generations g WHERE g.id=json_extract(payload,'$.guard.generation_id'))) OR
         (json_extract(payload,'$.guard.generation_id') IS NULL AND NOT EXISTS
          (SELECT 1 FROM fleets f WHERE f.key=json_extract(payload,'$.guard.fleet_key')
           AND f.incarnation=json_extract(payload,'$.guard.incarnation')))",
        vec![now.saturating_sub(7 * 86_400_000).into()])).await?;
    Ok(())
}
