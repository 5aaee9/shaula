//! Candidate scanning and per-revision bounded retry scheduling.
use crate::fleet_tasks::FleetTasks;
use shaula_core::{
    auth_validation::AuthValidationFactory, error::CoreResult, ports::Clock,
    registry::ControlPlaneStore,
};
use std::{collections::HashMap, sync::Arc};

#[derive(Default)]
pub struct AuthWorkerScheduler {
    deferred: Arc<tokio::sync::Mutex<HashMap<(String, i64), i64>>>,
}
impl AuthWorkerScheduler {
    pub async fn tick(
        &self,
        tasks: &mut FleetTasks,
        store: &Arc<dyn ControlPlaneStore>,
        clock: &Arc<dyn Clock>,
        factory: Arc<dyn AuthValidationFactory>,
        now: i64,
    ) -> CoreResult<()> {
        for key in store.auth_profile_keys().await? {
            // G1: the REAL Profile key reaches the worker — the namespaced
            // `auth/{key}` string is only the internal task identity. The
            // deferral gate is keyed by the CANDIDATE REF (profile +
            // desired revision, G2), so a new publication is never
            // stranded by an old revision's deadline.
            let Some(head) = store.auth_profile_get(&key).await? else {
                continue;
            };
            if head.status != "Validating" {
                continue;
            }
            let worker_key = format!("auth/{key}");
            if tasks.contains(&worker_key) {
                continue;
            }
            let gate_key = (key.clone(), head.desired_revision);
            if let Some(retry_at) = self.deferred.lock().await.get(&gate_key) {
                if now < *retry_at {
                    continue;
                }
            }
            // A new desired revision invalidates stale deferral state from
            // earlier revisions of this profile.
            self.deferred.lock().await.retain(|(profile, revision), _| {
                profile != &key || *revision == head.desired_revision
            });
            let deferred = self.deferred.clone();
            let store = store.clone();
            let clock = clock.clone();
            let factory = factory.clone();
            let real_key = key.clone();
            tasks.spawn(worker_key, async move {
                let flow =
                    super::validate(store, clock.clone(), real_key, gate_key.1, factory.as_ref())
                        .await;
                let mut gate = deferred.lock().await;
                match &flow {
                    Ok(super::WorkerFlow::Deferred { retry_at_unix_ms }) => {
                        // G2: None means bounded normal backoff — never an
                        // infinite deadline. Deadlines are ABSOLUTE unix
                        // ms, so the default backoff anchors at `now`.
                        gate.insert(
                            gate_key,
                            retry_at_unix_ms.unwrap_or_else(|| {
                                clock
                                    .now_unix_ms()
                                    .saturating_add(super::DEFAULT_RETRY_BACKOFF_MS)
                            }),
                        );
                    }
                    Err(_) => {
                        gate.insert(
                            gate_key,
                            clock
                                .now_unix_ms()
                                .saturating_add(super::DEFAULT_RETRY_BACKOFF_MS),
                        );
                    }
                    Ok(super::WorkerFlow::Done) => {
                        gate.remove(&gate_key);
                    }
                }
                flow.map(|_| ())
            });
        }
        Ok(())
    }
}
