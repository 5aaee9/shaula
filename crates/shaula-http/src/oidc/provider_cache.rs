//! Provider state shared by every authenticated request.
//!
//! Requests read an immutable [`ProviderSnapshot`] without waiting on anything
//! but a short synchronous lock. Only requests that actually need a refresh
//! (expired snapshot, unknown `kid`, or a failed previous refresh) take the
//! single-flight refresh lock, so a slow identity provider never stalls
//! requests that can be served from the current snapshot.
use super::{provider::ProviderSnapshot, AuthError, OidcConfig};
use std::{
    sync::{Arc, Mutex, PoisonError, RwLock},
    time::{Duration, Instant},
};

const BACKOFF: Duration = Duration::from_secs(10);

/// Why a caller may need fresher provider state than the current snapshot.
#[derive(Clone, Copy)]
pub(super) enum Need<'a> {
    /// Verify a token signed with this `kid`; refresh when it is not advertised.
    Kid(&'a str),
    /// Use metadata; retry discovery when the previous refresh failed.
    Healthy,
}

struct RefreshState {
    /// Start of the most recent refresh attempt; `None` until one happens.
    attempted: Option<Instant>,
    failed: bool,
}

pub(super) struct Provider {
    snapshot: RwLock<Arc<ProviderSnapshot>>,
    state: Mutex<RefreshState>,
    /// Single-flight guard: held for the duration of one discovery + JWKS fetch.
    refresh: tokio::sync::Mutex<()>,
}

impl Provider {
    pub fn new(snapshot: ProviderSnapshot) -> Self {
        Self {
            snapshot: RwLock::new(Arc::new(snapshot)),
            state: Mutex::new(RefreshState {
                attempted: None,
                failed: false,
            }),
            refresh: tokio::sync::Mutex::new(()),
        }
    }

    pub fn snapshot(&self) -> Arc<ProviderSnapshot> {
        self.snapshot
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn failed(&self) -> bool {
        self.state().failed
    }

    fn state(&self) -> std::sync::MutexGuard<'_, RefreshState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Returns `true` when `snapshot` can serve `need` without a refresh.
    fn satisfies(&self, snapshot: &ProviderSnapshot, need: Need<'_>) -> bool {
        !snapshot.expired()
            && match need {
                Need::Kid(kid) => snapshot.knows(kid),
                Need::Healthy => !self.failed(),
            }
    }

    /// Returns provider state usable for `need`, refreshing it first when
    /// required. Concurrent callers needing a refresh share one fetch: the
    /// first performs it while the rest wait and then re-evaluate against the
    /// result, falling under the backoff window like any later caller would.
    pub async fn current(
        &self,
        http: &reqwest::Client,
        config: &OidcConfig,
        need: Need<'_>,
    ) -> Result<Arc<ProviderSnapshot>, AuthError> {
        let snapshot = self.snapshot();
        if self.satisfies(&snapshot, need) {
            return Ok(snapshot);
        }
        let _flight = self.refresh.lock().await;
        let snapshot = self.snapshot();
        if self.satisfies(&snapshot, need) {
            return Ok(snapshot);
        }
        {
            let mut state = self.state();
            if state.attempted.is_some_and(|at| at.elapsed() < BACKOFF) {
                return if snapshot.expired() || state.failed {
                    Err(AuthError::Provider)
                } else {
                    Ok(snapshot)
                };
            }
            state.attempted = Some(Instant::now());
        }
        match ProviderSnapshot::load(http, config).await {
            Ok(fresh) => {
                let fresh = Arc::new(fresh);
                *self
                    .snapshot
                    .write()
                    .unwrap_or_else(PoisonError::into_inner) = fresh.clone();
                let mut state = self.state();
                state.attempted = Some(Instant::now());
                state.failed = false;
                Ok(fresh)
            }
            Err(error) => {
                self.state().failed = true;
                Err(error)
            }
        }
    }

    /// Test hook: pretend the last refresh attempt predates the backoff window.
    #[cfg(test)]
    pub fn elapse_backoff(&self) {
        self.state().attempted = None;
    }

    /// Test hook: age the current snapshot past its TTL.
    #[cfg(test)]
    pub fn expire_snapshot(&self) {
        let mut slot = self
            .snapshot
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        let mut aged = ProviderSnapshot::clone(&slot);
        aged.loaded -= super::provider::TTL;
        *slot = Arc::new(aged);
    }
}
