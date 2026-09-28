//! Terminal receipt, backend seal and occupancy release share one commit.
use sea_orm::{ConnectionTrait, DatabaseTransaction};
use shaula_core::{
    state_backend::{StateClaim, StateError, StateResult},
    worker::{CompletionKind, CompletionReceipt, ControlAccess},
};
use uuid::Uuid;

use super::{row::Row, sql, unavailable, worker::WorkerRow, SqliteStateBackend};

impl SqliteStateBackend {
    /// Daemon-only completion after its backend-specific removal gate. Worker
    /// booleans and an empty state alone never establish a cleanup result.
    pub async fn complete_worker(
        &self,
        access: &ControlAccess,
        request_id: Uuid,
        expected_revision: i64,
        kind: CompletionKind,
        now: i64,
    ) -> StateResult<CompletionReceipt> {
        let id = access.claim.generation_id.to_string();
        let tx = self.writer(&id).await?;
        let worker = WorkerRow::load(&tx, access).await?;
        let receipt = complete_on(
            &tx,
            &access.claim,
            worker,
            request_id,
            expected_revision,
            kind,
            now,
        )
        .await?;
        tx.commit().await.map_err(unavailable)?;
        Ok(receipt)
    }

    /// The existing daemon removal path calls this at its terminal transition.
    /// Returning false selects the existing legacy/static-test persistence path.
    pub(crate) async fn complete_generation(&self, id: &str, now: i64) -> StateResult<bool> {
        let tx = self.writer(id).await?;
        let Some(worker) = WorkerRow::read(&tx, id).await? else {
            return Ok(false);
        };
        if worker.completion_receipt.is_some() {
            return Ok(true);
        }
        let state = Row::load(&tx, id).await?;
        let claim = StateClaim {
            generation_id: Uuid::parse_str(id).map_err(|_| StateError::Invalid)?,
            worker_epoch: state.worker_epoch,
            worker_attempt: Uuid::parse_str(&state.worker_attempt)
                .map_err(|_| StateError::Unavailable)?,
        };
        let kind = if state.create_started {
            CompletionKind::ProviderCleanup
        } else {
            CompletionKind::NeverStarted
        };
        complete_on(
            &tx,
            &claim,
            worker,
            Uuid::new_v4(),
            state.revision,
            kind,
            now,
        )
        .await?;
        tx.commit().await.map_err(unavailable)?;
        Ok(true)
    }
}

async fn complete_on(
    tx: &DatabaseTransaction,
    claim: &StateClaim,
    worker: WorkerRow,
    request_id: Uuid,
    expected_revision: i64,
    kind: CompletionKind,
    now: i64,
) -> StateResult<CompletionReceipt> {
    let id = claim.generation_id.to_string();
    if let Some(saved) = &worker.completion_receipt {
        let receipt: CompletionReceipt =
            serde_json::from_str(saved).map_err(unavailable_receipt)?;
        if worker.completion_request.as_deref() == Some(&request_id.to_string())
            && receipt.state_revision == expected_revision
            && receipt.kind == kind
            && receipt.claim == *claim
        {
            return Ok(receipt);
        }
        return Err(StateError::Conflict);
    }
    if worker.handover.is_some()
        || worker.active_command.is_some()
        || !matches!(worker.phase.as_str(), "running" | "fenced" | "admitted")
    {
        return Err(StateError::Conflict);
    }
    let state = Row::load(tx, &id).await?;
    state.writable()?;
    if state.revision != expected_revision || state.lock(tx).await?.is_some() {
        return Err(StateError::Conflict);
    }
    let snapshot = state.snapshot(tx).await?.ok_or(StateError::Unavailable)?;
    if !snapshot.document.managed_empty() {
        return Err(StateError::Conflict);
    }
    let generation = tx
        .query_one(sql(
            "SELECT state, jit_phase FROM runner_generations WHERE id = ?",
            vec![id.clone().into()],
        ))
        .await
        .map_err(unavailable)?
        .ok_or(StateError::Conflict)?;
    // Destroying is entered by the daemon only after its existing ordinary
    // removal or resource-first hard-lifetime registration gates resolve.
    if generation
        .try_get::<String>("", "state")
        .map_err(unavailable)?
        != "Destroying"
    {
        return Err(StateError::Conflict);
    }
    let operations = tx
        .query_all(sql(
            "SELECT kind, state FROM runner_operations WHERE generation_id = ?",
            vec![id.clone().into()],
        ))
        .await
        .map_err(unavailable)?;
    let mut create = false;
    for operation in operations {
        let operation_kind: String = operation.try_get("", "kind").map_err(unavailable)?;
        // Historical failed operations retain their starting facts. Current
        // command/handover authority above, not an old operation label, proves
        // quiescence; cleanup must attest the exact current state revision.
        create |= operation_kind == "Create";
    }
    match kind {
        CompletionKind::NeverStarted if !state.create_started && !create => {}
        CompletionKind::ProviderCleanup
            if state.create_started && worker.cleanup_revision == Some(state.revision) => {}
        // Existing explicit operator attestation remains a separate trusted
        // maintenance operation, never forgeable through automatic completion.
        _ => return Err(StateError::Conflict),
    }
    let receipt = CompletionReceipt {
        claim: claim.clone(),
        request_id,
        state_revision: expected_revision,
        kind,
        completed_at: now,
    };
    let encoded = serde_json::to_string(&receipt).map_err(unavailable_receipt)?;
    tx.execute(sql(
        "UPDATE generation_http_state SET sealed = 1 WHERE generation_id = ?",
        vec![id.clone().into()],
    ))
    .await
    .map_err(unavailable)?;
    tx.execute(sql(
            "UPDATE lifecycle_workers SET phase = 'completed', completion_request = ?, completion_receipt = ? WHERE generation_id = ?",
            vec![request_id.to_string().into(), encoded.into(), id.clone().into()],
        )).await.map_err(unavailable)?;
    tx.execute(sql(
        "UPDATE runner_generations SET state = 'Destroyed', updated_at = ? WHERE id = ?",
        vec![now.into(), id.into()],
    ))
    .await
    .map_err(unavailable)?;
    Ok(receipt)
}

fn unavailable_receipt(_: serde_json::Error) -> StateError {
    StateError::Unavailable
}
