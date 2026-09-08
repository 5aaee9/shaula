//! Durable apply-start intent sink: the Template Runtime calls this with
//! the plan provenance BEFORE a mutating apply may spawn, so
//! `ApplyStarting`/`DestroyApplyStarting` is on disk first (spec 0004
//! section 6, at-most-once).

use std::sync::Arc;

use shaula_core::ports::{ApplyClaim, ApplyIntentSink, PlanProvenance};
use shaula_core::registry::LifecycleStore;

use crate::effect_gate::FleetEffectGates;

/// Persists provenance into the runner-operations ledger and takes the
/// fleet's shared effect admission claim.
pub struct LedgerApplyIntentSink {
    pub store: Arc<dyn LifecycleStore>,
    pub gates: Arc<FleetEffectGates>,
}

#[async_trait::async_trait]
impl ApplyIntentSink for LedgerApplyIntentSink {
    async fn persist_apply_starting(
        &self,
        provenance: &PlanProvenance,
    ) -> Result<ApplyClaim, String> {
        // R5-02/R6-03: the gate claim is taken BEFORE the durable record
        // and held by the caller only until the SPAWN HANDOVER (the
        // EngineSpawn returned by start_apply_saved_plan) — never for
        // the child's whole lifetime. While it is held, a decommission
        // or a head-advancing PUT — both of which take the gate
        // exclusively around their commits — cannot interleave; and any
        // decommission that already committed makes the CAS's
        // in-transaction fleet-head fence below fail, so no apply can
        // spawn against a deleted fleet or a stale desired revision.
        let record = self
            .store
            .generation_get(&provenance.generation_id)
            .await
            .map_err(|e| e.summary)?
            .ok_or_else(|| {
                format!(
                    "generation {} is not in the ledger",
                    provenance.generation_id
                )
            })?;
        let claim = self.gates.acquire_claim(&record.fleet_key).await;

        // The FENCE lives inside the store's single transaction (F05):
        // a Create is recorded only while the fleet head still matches
        // the generation's admitted revision and no deletion marker is
        // set; a Destroy is fenced only on generation existence so a
        // DELETE/replacement can never block cleanup of the generation's
        // own original resources (spec 0002 §8.330/§8.334).
        self.store
            .operation_record_apply_starting(
                provenance,
                "tfplan",
                chrono::Utc::now().timestamp_millis(),
            )
            .await
            .map_err(|e| e.summary)?;
        Ok(Box::new(claim))
    }
}

/// One runtime call's admitted intents. A definite result closes only these
/// exact attempts; an error, cancellation or crash leaves their durable rows open.
pub(crate) struct TrackedApplyIntentSink {
    inner: Arc<dyn ApplyIntentSink>,
    attempts: std::sync::Mutex<Vec<String>>,
}

impl TrackedApplyIntentSink {
    pub(crate) fn new(inner: Arc<dyn ApplyIntentSink>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            attempts: std::sync::Mutex::new(Vec::new()),
        })
    }

    pub(crate) async fn complete(
        &self,
        store: &dyn LifecycleStore,
        now: i64,
    ) -> shaula_core::error::CoreResult<()> {
        let attempts = self
            .attempts
            .lock()
            .map_err(|_| {
                shaula_core::error::CoreError::new(
                    shaula_core::error::ReasonCode::Internal,
                    "apply attempt tracking unavailable",
                )
            })?
            .clone();
        for id in attempts {
            store.operation_update_state(&id, "Succeeded", now).await?;
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl ApplyIntentSink for TrackedApplyIntentSink {
    async fn persist_apply_starting(
        &self,
        provenance: &PlanProvenance,
    ) -> Result<ApplyClaim, String> {
        let claim = self.inner.persist_apply_starting(provenance).await?;
        self.attempts
            .lock()
            .map_err(|_| "apply attempt tracking unavailable".to_string())?
            .push(provenance.attempt_id.clone());
        Ok(claim)
    }
}
