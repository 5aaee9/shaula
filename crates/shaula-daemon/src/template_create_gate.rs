//! Per-template Create serialization. Some platforms lock a shared source
//! while cloning (Proxmox locks the template VM's config), so concurrent
//! Creates from one template can fail instead of waiting. A failed Create
//! with a partial state is quarantined by design, so contention must be
//! prevented before the apply rather than recovered after it.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;

/// Optional bound on concurrent Create applies per Template Profile, shared
/// by every Fleet using that Template. `None` leaves Creates bounded only by
/// the global create scheduler.
#[derive(Clone, Default)]
pub struct TemplateCreateGates {
    limit: Option<usize>,
    gates: Arc<Mutex<HashMap<String, Arc<Semaphore>>>>,
}

impl TemplateCreateGates {
    pub fn new(limit: Option<usize>) -> Self {
        Self {
            limit,
            gates: Arc::default(),
        }
    }

    /// The gate for `template`, or `None` when Creates are not bounded per
    /// Template. One semaphore per Template key lives for the daemon's life.
    pub fn gate(&self, template: &str) -> Option<Arc<Semaphore>> {
        let limit = self.limit?;
        let mut gates = self.gates.lock().unwrap_or_else(|e| e.into_inner());
        Some(
            gates
                .entry(template.to_owned())
                .or_insert_with(|| Arc::new(Semaphore::new(limit)))
                .clone(),
        )
    }
}
