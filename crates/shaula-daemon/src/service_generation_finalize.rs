//! Operator finalization of Quarantined generations (spec 0028): the
//! ledger-only `Quarantined -> Destroyed` transition after out-of-band
//! verification, never a remote effect.

use super::{unprocessable, ControlPlane};
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::registry::{Actor, ChangeView, MutationAccepted, MutationError, Scope};

const MAX_REASON_BYTES: usize = 4096;

impl ControlPlane {
    pub(crate) async fn generation_finalize_impl(
        &self,
        actor: &Actor,
        generation_id: &str,
        reason: &str,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>> {
        if !actor.has(Scope::FleetRetire) {
            return Ok(Err(unprocessable(
                ReasonCode::SpecInvalid,
                "missing fleet.retire scope",
            )));
        }
        let trimmed = reason.trim();
        if trimmed.is_empty() {
            return Ok(Err(unprocessable(
                ReasonCode::SpecInvalid,
                "finalize requires a non-empty reason describing the external verification",
            )));
        }
        if reason.len() > MAX_REASON_BYTES {
            return Ok(Err(unprocessable(
                ReasonCode::SpecInvalid,
                "finalize reason exceeds 4096 bytes",
            )));
        }
        let canonical_body = format!("finalize:{reason}");
        match self
            .idempotency_replay(
                "runner_generation",
                generation_id,
                &idempotency_key,
                &canonical_body,
                "none",
            )
            .await?
        {
            Err(conflict) => return Ok(Err(conflict)),
            Ok(Some(accepted)) => return Ok(Ok(accepted)),
            Ok(None) => {}
        }

        let Some(generation) = self.store.generation_lookup(generation_id).await? else {
            return Ok(Err(MutationError::NotFound));
        };
        if !generation.state.is_quarantined() {
            return Ok(Err(MutationError::Conflict {
                summary: format!(
                    "generation is {:?}, not Quarantined; finalize only terminates quarantined rows",
                    generation.state
                ),
            }));
        }
        // Already-finalized replays resolve through idempotency or the
        // state check above; a repeated call without a key is a 409, not
        // a silent success.
        let now = self.now_ms();
        let change_id = self.new_id();
        let accepted = MutationAccepted {
            etag: format!("{}:{}", generation.id, 1),
            change: ChangeView {
                id: change_id.clone(),
                resource_kind: "runner_generation".into(),
                resource_key: generation_id.to_string(),
                revision: 0,
                kind: "Finalize".into(),
                state: "Succeeded".into(),
                reason: Some(reason.to_string()),
            },
            no_op: false,
        };

        let idempotency = match idempotency_key.map(|idem| {
            let request_hash = self.idempotency_hash(
                "runner_generation",
                generation_id,
                &idem,
                &canonical_body,
                "none",
            );
            let response_body = serde_json::to_string(&accepted)
                .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?;
            Ok(shaula_core::registry::IdempotencyInsert {
                id: format!("idem-finalize-{}", self.new_id()),
                resource_kind: "runner_generation".to_string(),
                resource_key: generation_id.to_string(),
                idempotency_key: idem,
                request_hash,
                response_status: 202,
                response_body: Some(response_body),
                now,
            })
        }) {
            Some(Ok(value)) => Some(value),
            Some(Err(e)) => return Err(e),
            None => None,
        };

        // The fleet's effect gate serializes finalize against in-flight
        // supervisor work on the same fleet, so a stale quarantine
        // admission cannot be re-armed by a racing tick.
        let effect_gate = self
            .effect_gates
            .acquire_exclusive(&generation.fleet_key)
            .await;
        let committed = self
            .store
            .commit_generation_finalize(generation_id, &actor.name, reason, idempotency, now)
            .await;
        drop(effect_gate);
        if let Err(fence) = committed? {
            return Ok(Err(fence));
        }
        Ok(Ok(accepted))
    }
}
