use super::*;
use sha2::{Digest, Sha256};
use shaula_core::{
    state_backend::{StateError, StateResult},
    worker::{wire::*, ControlCapability, PROTOCOL_VERSION},
};
use uuid::Uuid;

#[async_trait::async_trait]
impl WorkerControl for Workers {
    async fn logs(
        &self,
        generation: Uuid,
        token: &ControlCapability,
        request: LogRequest,
    ) -> StateResult<LogResponse> {
        self.write_log(generation, token, request).await
    }
    async fn authenticate(
        &self,
        generation: Uuid,
        capability: &ControlCapability,
    ) -> StateResult<()> {
        let session = self.session(generation).await?;
        if !capability.matches(&session.access.capability.verifier()) {
            return Err(StateError::Unauthorized);
        }
        self.journal.authenticate(&session.access).await
    }

    async fn material(
        &self,
        generation: Uuid,
        capability: &ControlCapability,
        id: Uuid,
    ) -> StateResult<Vec<u8>> {
        self.authenticate(generation, capability).await?;
        let session = self.session(generation).await?;
        let state = session.state.lock().await;
        let (saved, bytes) = state.material.as_ref().ok_or(StateError::Conflict)?;
        if id != *saved {
            return Err(StateError::Conflict);
        }
        Ok(bytes.clone())
    }

    async fn call(
        &self,
        capability: &ControlCapability,
        request: ControlRequest,
    ) -> StateResult<ControlResponse> {
        self.authenticate(request.claim.generation_id, capability)
            .await?;
        let session = self.session(request.claim.generation_id).await?;
        if request.claim != session.access.claim {
            return Err(StateError::Unauthorized);
        }
        let bytes = serde_json::to_vec(&request).map_err(|_| StateError::Invalid)?;
        if bytes.len() > MAX_CONTROL_BYTES {
            return Err(StateError::TooLarge);
        }
        let digest = hex::encode(Sha256::digest(&bytes));
        let mut state = session.state.lock().await;
        if let Some((saved, response)) = state.replay.get(&request.request_id) {
            if saved != &digest {
                return Err(StateError::Conflict);
            }
            return serde_json::from_slice(response).map_err(|_| StateError::Unavailable);
        }
        if matches!(request.message, ControlMessage::Poll) {
            if let Some(receipt) = self.journal.receipt(&session.access).await? {
                return Ok(ControlResponse::Desired(Directive::Complete(receipt)));
            }
            let desired = if self.stopping.load(Ordering::Acquire) {
                Directive::Quiesce
            } else {
                match &state.desired {
                    Some(Directive::Create(value)) => Directive::Create(value.clone()),
                    Some(Directive::Destroy(value)) => Directive::Destroy(value.clone()),
                    _ => Directive::Wait,
                }
            };
            return Ok(ControlResponse::Desired(desired));
        }
        // Control effects per attempt are bounded; read-only polls never grow
        // the replay table. Exhaustion preserves uncertainty instead of evicting
        // an old effect identity and accidentally accepting it again.
        if state.replay.len() >= 256 {
            return Err(StateError::TooLarge);
        }
        match request.message {
            ControlMessage::Handshake { protocol } if protocol == PROTOCOL_VERSION => {}
            ControlMessage::Prepared(result) => state.prepared(result)?,
            ControlMessage::ApplyStarting(proof) => {
                if proof.generation_id != request.claim.generation_id.to_string()
                    || state.guard.is_some()
                {
                    return Err(StateError::Conflict);
                }
                let sink = state.sink.clone().ok_or(StateError::Conflict)?;
                state.guard = Some(
                    sink.persist_apply_starting(&proof)
                        .await
                        .map_err(|_| StateError::Conflict)?,
                );
            }
            ControlMessage::Spawned(proof) => {
                if proof.generation_id != request.claim.generation_id.to_string() {
                    return Err(StateError::Conflict);
                }
                self.journal
                    .spawn_ack(&session.access, &proof.attempt_id)
                    .await?;
                state.guard = None;
            }
            ControlMessage::CommandEnded(proof) => {
                if proof.generation_id != request.claim.generation_id.to_string() {
                    return Err(StateError::Conflict);
                }
                self.journal
                    .command_ended(&session.access, &proof.attempt_id)
                    .await?;
            }
            ControlMessage::BootstrapStarting(proof) => {
                if proof.generation_id != request.claim.generation_id.to_string()
                    || state.bootstrap_guard.is_some()
                {
                    return Err(StateError::Conflict);
                }
                let sink = state.sink.clone().ok_or(StateError::Conflict)?;
                state.bootstrap_guard = Some(
                    sink.authorize_bootstrap(&proof)
                        .await
                        .map_err(|_| StateError::Conflict)?,
                );
            }
            ControlMessage::BootstrapEnded(proof) => {
                if proof.generation_id != request.claim.generation_id.to_string() {
                    return Err(StateError::Conflict);
                }
                self.journal
                    .command_ended(&session.access, &format!("bootstrap:{}", proof.attempt_id))
                    .await?;
                state.bootstrap_guard = None;
            }
            ControlMessage::CreateFinished {
                material_id,
                result,
            } => {
                if !matches!(&state.desired, Some(Directive::Create(value)) if value.id == material_id)
                    || state.guard.is_some()
                    || state.bootstrap_guard.is_some()
                {
                    return Err(StateError::Conflict);
                }
                let sender = state.created_result.take().ok_or(StateError::Conflict)?;
                state.desired = None;
                state.material = None;
                let _ = sender.send(result);
            }
            ControlMessage::DestroyFinished {
                material_id,
                result,
            } => {
                if !matches!(&state.desired, Some(Directive::Destroy(value)) if value.id == material_id)
                    || state.guard.is_some()
                    || state.bootstrap_guard.is_some()
                {
                    return Err(StateError::Conflict);
                }
                if result.is_ok() {
                    self.journal.verify_cleanup(&session.access).await?;
                }
                let sender = state.destroyed_result.take().ok_or(StateError::Conflict)?;
                state.desired = None;
                state.material = None;
                let _ = sender.send(result);
            }
            _ => return Err(StateError::Invalid),
        }
        let response = ControlResponse::Ack;
        state.replay.insert(
            request.request_id,
            (
                digest,
                serde_json::to_vec(&response).map_err(|_| StateError::Unavailable)?,
            ),
        );
        Ok(response)
    }
}
