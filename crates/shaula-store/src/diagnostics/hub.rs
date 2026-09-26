use shaula_core::diagnostics::*;
use shaula_observability::{MetricOperation, MetricResult, TelemetryHandle};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::mpsc;

pub(super) struct Entry {
    pub key: String,
    pub next: u64,
    pub accepted: u64,
    pub payload: String,
    pub invalid: bool,
    pub health: u64,
    pub conflict: bool,
}
pub(crate) struct Hub {
    pub(super) entries: Mutex<HashMap<String, Entry>>,
    sender: mpsc::Sender<DecisionObservation>,
    pub dropped: AtomicU64,
    health: AtomicU64,
    pub write_failures: AtomicU64,
}
impl Hub {
    pub fn start(db: sea_orm::DatabaseConnection) -> Arc<Self> {
        let (sender, mut receiver) = mpsc::channel(1024);
        let hub = Arc::new(Self {
            entries: Mutex::new(HashMap::new()),
            sender,
            health: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            write_failures: AtomicU64::new(0),
        });
        let weak = Arc::downgrade(&hub);
        tokio::spawn(async move {
            let mut retention = tokio::time::interval_at(
                tokio::time::Instant::now() + std::time::Duration::from_secs(60),
                std::time::Duration::from_secs(60),
            );
            loop {
                let observation = tokio::select! {
                    next = receiver.recv() => { let Some(next) = next else { break }; next },
                    _ = retention.tick() => {
                        if weak.strong_count() == 0 { break; }
                        let _ = super::writer::collect(&db, chrono::Utc::now().timestamp_millis()).await;
                        continue;
                    }
                };
                let Some(hub) = weak.upgrade() else { break };
                if !hub.current(&observation) {
                    continue;
                }
                if super::writer::write(&db, &hub, &observation).await.is_err() {
                    hub.write_failures.fetch_add(1, Ordering::Relaxed);
                    TelemetryHandle::new().record(
                        MetricOperation::DiagnosticWrite,
                        MetricResult::Failed,
                        1,
                    );
                    hub.invalidate(&observation.epoch);
                }
            }
        });
        hub
    }
    pub fn current(&self, o: &DecisionObservation) -> bool {
        self.entries
            .lock()
            .ok()
            .and_then(|entries| {
                entries.get(&o.epoch).map(|e| {
                    !e.invalid
                        && e.accepted == o.sequence
                        && e.health == self.health.load(Ordering::Relaxed)
                })
            })
            .unwrap_or(false)
    }
    pub fn invalid_reason(&self, epoch: &str) -> Code {
        if self
            .entries
            .lock()
            .ok()
            .and_then(|entries| entries.get(epoch).map(|e| e.conflict))
            .unwrap_or(false)
        {
            Code::ObservationConflict
        } else {
            Code::ObservationMissing
        }
    }
    fn invalidate(&self, epoch: &str) {
        if let Ok(mut entries) = self.entries.try_lock() {
            if let Some(entry) = entries.get_mut(epoch) {
                entry.invalid = true;
            }
        } else {
            self.health.fetch_add(1, Ordering::Relaxed);
        }
    }
}
impl DiagnosticSink for Hub {
    fn register(&self, guard: &Guard, lane: Lane, question: QuestionId) -> Option<String> {
        let key = format!("{}:{lane:?}:{question:?}", super::subject_key(guard));
        let mut entries = self
            .entries
            .try_lock()
            .map_err(|_| self.health.fetch_add(1, Ordering::Relaxed))
            .ok()?;
        entries.retain(|_, entry| entry.key != key);
        // Each registration retires its predecessor. Overflow drops optional
        // process evidence instead of retaining unbounded per-object state.
        if entries.len() >= 4096 {
            entries.clear();
            self.dropped.fetch_add(1, Ordering::Relaxed);
            TelemetryHandle::new().record(
                MetricOperation::DiagnosticDrop,
                MetricResult::Degraded,
                1,
            );
        }
        let epoch = uuid::Uuid::new_v4().to_string();
        entries.insert(
            epoch.clone(),
            Entry {
                key,
                next: 0,
                accepted: 0,
                payload: String::new(),
                invalid: true,
                conflict: false,
                health: self.health.load(Ordering::Relaxed),
            },
        );
        Some(epoch)
    }
    fn sequence(&self, epoch: &str) -> Option<u64> {
        let mut entries = self
            .entries
            .try_lock()
            .map_err(|_| self.health.fetch_add(1, Ordering::Relaxed))
            .ok()?;
        let entry = entries.get_mut(epoch)?;
        entry.next = entry
            .next
            .checked_add(1)
            .filter(|v| *v <= i64::MAX as u64)?;
        Some(entry.next)
    }
    fn publish(&self, observation: DecisionObservation) {
        if observation.version != 1 {
            self.invalidate(&observation.epoch);
            return;
        }
        let encoded = serde_json::to_string(&observation);
        let Ok(payload) = encoded else {
            self.invalidate(&observation.epoch);
            return;
        };
        if payload.len() > 65536 {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            TelemetryHandle::new().record(
                MetricOperation::DiagnosticDrop,
                MetricResult::Degraded,
                1,
            );
            self.invalidate(&observation.epoch);
            return;
        }
        let Ok(mut entries) = self.entries.try_lock() else {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            TelemetryHandle::new().record(
                MetricOperation::DiagnosticDrop,
                MetricResult::Degraded,
                1,
            );
            // Poison/contention must not leave old evidence usable.
            self.invalidate(&observation.epoch);
            return;
        };
        let Some(entry) = entries.get_mut(&observation.epoch) else {
            return;
        };
        if observation.sequence < entry.accepted {
            return;
        }
        if observation.sequence == entry.accepted {
            if payload != entry.payload {
                entry.invalid = true;
                entry.conflict = true;
            }
            return;
        }
        entry.accepted = observation.sequence;
        entry.payload = payload;
        entry.invalid = false;
        entry.conflict = false;
        entry.health = self.health.load(Ordering::Relaxed);
        if self.sender.try_send(observation).is_err() {
            entry.invalid = true;
            self.dropped.fetch_add(1, Ordering::Relaxed);
            TelemetryHandle::new().record(
                MetricOperation::DiagnosticDrop,
                MetricResult::Degraded,
                1,
            );
        }
    }
}

#[cfg(test)]
#[path = "hub_tests.rs"]
mod tests;
