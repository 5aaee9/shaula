//! Shared per-fleet admission gate for external effects (R5-02/R6-03).
//!
//! Every mutating apply (Create/Destroy) holds a SHARED claim from its
//! durable `ApplyStarting` record through the SPAWN HANDOVER (the point
//! where the engine process exists under its own process-tree fence,
//! R6-04) — not for the child's whole lifetime. A fleet decommission
//! takes the gate EXCLUSIVELY around its deletion commit, and a
//! head-advancing Fleet PUT around its mutation commit (R6-02). The
//! sides therefore serialize: no PUT/DELETE can commit between an
//! apply's durable admission and its spawn, and an apply admitted after
//! such a commit fails the CAS's in-transaction fleet-head fence
//! instead of spawning against a stale desired state.
//!
//! Lock ordering is uniform (gate before database), so the gate cannot
//! deadlock against ledger transactions.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

/// Per-fleet effect serialization. One gate per fleet; fleets are
/// independent. Entries are never removed — the daemon caps active
/// fleets, so the map is bounded.
#[derive(Default)]
pub struct FleetEffectGates {
    fleets: tokio::sync::Mutex<HashMap<String, Arc<RwLock<()>>>>,
}

impl FleetEffectGates {
    pub fn new() -> Self {
        Self::default()
    }

    async fn entry(&self, fleet_key: &str) -> Arc<RwLock<()>> {
        self.fleets
            .lock()
            .await
            .entry(fleet_key.to_string())
            .or_default()
            .clone()
    }

    /// Shared effect-side claim: many applies on one fleet hold it
    /// concurrently. This is the value boxed into the opaque
    /// [`shaula_core::ports::ApplyClaim`].
    pub async fn acquire_claim(&self, fleet_key: &str) -> OwnedRwLockReadGuard<()> {
        self.entry(fleet_key).await.read_owned().await
    }

    /// Exclusive decommission-side claim: waits for every in-flight
    /// apply on the fleet and blocks new claims until the deletion
    /// commit has landed.
    pub async fn acquire_exclusive(&self, fleet_key: &str) -> OwnedRwLockWriteGuard<()> {
        self.entry(fleet_key).await.write_owned().await
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[tokio::test]
    async fn shared_claims_coexist_per_fleet() {
        let gates = FleetEffectGates::new();
        let a = gates.acquire_claim("fleet-a").await;
        let b = gates.acquire_claim("fleet-a").await;
        let _other = gates.acquire_claim("fleet-b").await;
        drop((a, b));
    }

    #[tokio::test]
    async fn exclusive_waits_for_every_shared_claim() {
        let gates = Arc::new(FleetEffectGates::new());
        let claim = gates.acquire_claim("fleet-a").await;

        let waiter = {
            let gates = Arc::clone(&gates);
            tokio::spawn(async move { gates.acquire_exclusive("fleet-a").await })
        };
        // A queued writer blocks NEW shared claims too (tokio RwLock is
        // fair): the decommission must not be starved by applies that
        // were admitted after it started waiting.
        let blocked = {
            let gates = Arc::clone(&gates);
            tokio::spawn(async move { gates.acquire_claim("fleet-a").await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!waiter.is_finished(), "exclusive must wait for claims");
        assert!(!blocked.is_finished(), "new claims queue behind a writer");

        drop(claim);
        let _exclusive = waiter.await.unwrap();
        // tokio::spawn JoinHandle needs the guard moved out; dropping here
        // releases the exclusive side for the blocked reader.
        drop(_exclusive);
        let _admitted = blocked.await.unwrap();
    }

    #[tokio::test]
    async fn shared_waits_while_exclusive_is_held() {
        let gates = Arc::new(FleetEffectGates::new());
        let exclusive = gates.acquire_exclusive("fleet-a").await;

        let waiter = {
            let gates = Arc::clone(&gates);
            tokio::spawn(async move { gates.acquire_claim("fleet-a").await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!waiter.is_finished(), "claims wait for the decommission");

        drop(exclusive);
        let _claim = waiter.await.unwrap();
    }

    #[tokio::test]
    async fn independent_fleets_never_block_each_other() {
        let gates = FleetEffectGates::new();
        let _exclusive = gates.acquire_exclusive("fleet-a").await;
        let _claim = gates.acquire_claim("fleet-b").await;
    }
}
