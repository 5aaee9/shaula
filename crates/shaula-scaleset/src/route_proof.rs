//! Bounded route-proof freshness (spec 0011 §5.3): positive authorization
//! evidence for one exact target + context is reused for at most 60
//! seconds, negative evidence for at most 15. A refresh RE-CHECKS the
//! installation identity/suspension (App level), the target numeric
//! identity and the Actions access path — minting a new token alone
//! never extends a proof. Refreshed identity metadata is EVIDENCE to
//! compare against the pinned/persisted authorization, never permission
//! to replace a pin (F4). The state is owned by ONE client instance
//! (exact auth context + target): refresh concurrency is per-route
//! (the mutex covers only that route's refresh), never global.

use shaula_core::ports::{AccessFailure, RouteProof};
use std::sync::atomic::Ordering;

use crate::auth::Credential;
use crate::error::ScalesetError;
use crate::installation::InstallationLookup;
use crate::ScalesetClient;

/// Positive TTL: authorization evidence older than this is re-proven
/// before any new effect (spec 0011 §5.3).
pub const POSITIVE_TTL_MS: i64 = 60_000;
/// Negative TTL: a failed re-proof blocks new effects for at most this
/// long before the next refresh attempt.
pub const NEGATIVE_TTL_MS: i64 = 15_000;

/// The route identity the FIRST successful proof pinned (or that the
/// persisted context declared). Later refreshes must agree — a drifting
/// identity is a block, never a silent rebind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PinnedIdentity {
    pub installation_id: i64,
    pub account_id: i64,
    pub organization_id: Option<i64>,
    pub repository_id: Option<i64>,
    pub repository_owner_id: Option<i64>,
}

/// Freshness state of one exact route.
#[derive(Default)]
pub struct RouteProofState {
    positive: Option<(u64, RouteProof)>,
    pinned: Option<PinnedIdentity>,
    negative: Option<(u64, i64, AccessFailure)>,
}

impl ScalesetClient {
    pub(crate) async fn route_proof_failure_window_impl(
        &self,
        failure: &AccessFailure,
    ) -> Option<(i64, i64)> {
        let state = self.proof.lock().await;
        state.negative.as_ref().and_then(|(_, until, cached)| {
            (cached == failure).then_some((until.saturating_sub(NEGATIVE_TTL_MS), *until))
        })
    }

    /// Returns a valid route proof, refreshing when the positive window
    /// expired or a negative window elapsed. Time is read AFTER the lock
    /// so a queued waiter never consumes a stale decision, and a proof
    /// that expired while refreshing is never returned as valid.
    pub(crate) async fn ensure_route_proof_impl(&self) -> Result<RouteProof, AccessFailure> {
        let mut state = self.proof.lock().await;
        let now = self.clock.now_unix_ms();
        let epoch_before = self.proof_epoch.load(Ordering::SeqCst);
        if let Some((epoch, proof)) = &state.positive {
            if *epoch == epoch_before
                && proof.checked_at_unix_ms <= now
                && proof.valid_until_unix_ms > now
            {
                return Ok(*proof);
            }
        }
        if let Some((epoch, negative_until, failure)) = &state.negative {
            if *epoch == epoch_before && *negative_until > now {
                return Err(failure.clone());
            }
        }
        // G7: the freshness window is anchored at the START of evidence
        // collection — the oldest installation response bounds the proof.
        let evidence_started_at = self.clock.now_unix_ms();
        match self
            .refresh_route_proof(state.pinned, evidence_started_at)
            .await
        {
            Ok(proof)
                if self.proof_epoch.load(std::sync::atomic::Ordering::SeqCst) == epoch_before =>
            {
                // Re-read time AFTER the refresh completed: a proof whose
                // window already lapsed is not handed out as fresh.
                let finished_at = self.clock.now_unix_ms();
                if proof.valid_until_unix_ms <= finished_at || finished_at < evidence_started_at {
                    state.positive = None;
                    return Err(AccessFailure::Unavailable {
                        summary: "route evidence expired during refresh".into(),
                    });
                }
                state.pinned = Some(PinnedIdentity {
                    installation_id: proof.installation_id,
                    account_id: proof.account_id,
                    organization_id: proof.organization_id,
                    repository_id: proof.repository_id,
                    repository_owner_id: proof.repository_owner_id,
                });
                state.positive = Some((epoch_before, proof));
                state.negative = None;
                Ok(proof)
            }
            Ok(_) => {
                // An invalidation landed mid-refresh: refuse to publish —
                // the next attempt re-proves from scratch.
                state.positive = None;
                state.negative = None;
                Err(AccessFailure::Unavailable {
                    summary: "route invalidated during refresh".into(),
                })
            }
            Err(failure) => {
                state.positive = None;
                state.negative = Some((
                    self.proof_epoch.load(Ordering::SeqCst),
                    self.clock.now_unix_ms().saturating_add(NEGATIVE_TTL_MS),
                    failure.clone(),
                ));
                Err(failure)
            }
        }
    }

    /// Observed access failures invalidate the cached positive proof
    /// (spec 0011 §5.3): the next effect waits for a full re-proof. Never
    /// blocks on the lock — when a refresh is in flight it manages its
    /// own state.
    pub(crate) fn invalidate_route_proof(&self, failure: &ScalesetError) {
        if matches!(
            failure.status(),
            Some(401) | Some(403) | Some(404) | Some(429)
        ) {
            // G6: the epoch bump happens WITHOUT the refresh mutex — an
            // invalidation observed during another task's refresh is
            // never dropped. The refresh refuses to publish across a
            // bump.
            self.proof_epoch.fetch_add(1, Ordering::SeqCst);
            // Cached entries carry their publication epoch too. A bump
            // after the refresh's final check therefore invalidates them
            // even if this thread cannot acquire the refresh mutex.
        }
    }

    /// One full re-proof of the route: installation identity +
    /// suspension + scope permission (GitHub App credentials), target
    /// numeric identity, and the Actions access path — each compared
    /// against the pinned identity and the persisted expected context.
    async fn refresh_route_proof(
        &self,
        pinned: Option<PinnedIdentity>,
        evidence_started_at: i64,
    ) -> Result<RouteProof, AccessFailure> {
        let mut installation = None;
        if let Credential::GitHubApp {
            client_id,
            installation_id: bound_installation,
            private_key,
        } = self.admin.credential()
        {
            // App-level re-check: the BOUND installation must still exist,
            // still answer for THIS App and not be suspended. An
            // uninstall+reinstall (new id) never silently passes here.
            let resolver = crate::installation::AppInstallationResolver::new(
                self.config.github_api_base.clone(),
                self.http.clone(),
                self.clock.clone(),
            );
            let lookup = resolver
                .installation_by_id(
                    client_id,
                    private_key,
                    *bound_installation,
                    &self.config.target,
                )
                .await;
            match lookup {
                InstallationLookup::Proven(proof) => {
                    // The scope permission was re-checked against the
                    // FRESH response (F4): a revoked runner permission
                    // fails the proof.
                    if !proof.has_required_permission {
                        return Err(AccessFailure::PermissionDenied);
                    }
                    installation = Some(proof);
                }
                InstallationLookup::NotFound | InstallationLookup::IdentityMismatch => {
                    return Err(AccessFailure::Unauthenticated);
                }
                InstallationLookup::Suspended => {
                    return Err(AccessFailure::PermissionDenied);
                }
                InstallationLookup::PermissionDenied => {
                    return Err(AccessFailure::PermissionDenied)
                }
                InstallationLookup::Transient {
                    retry_after_ms: Some(ms),
                } => {
                    return Err(AccessFailure::RateLimited {
                        retry_after: Some(std::time::Duration::from_millis(ms.max(0) as u64)),
                    });
                }
                InstallationLookup::Transient { .. } => {
                    return Err(AccessFailure::Unavailable {
                        summary: "installation re-proof unavailable".into(),
                    });
                }
            }
        }
        // Target numeric identity: repository/organization ids re-proven.
        let identity = self
            .resolve_target_identity_impl(&self.config.target)
            .await?;
        self.verify_route_identity(installation.as_ref(), &identity)?;
        let installation_id = installation.as_ref().map_or(0, |p| p.installation_id);
        let account_id = installation.as_ref().map_or(0, |p| p.account_id);
        // F4: the refreshed metadata must agree with the pinned identity
        // AND the persisted expected context — it is evidence to compare,
        // not permission to replace a pin.
        if let Some(pinned) = pinned {
            if pinned.installation_id != installation_id
                || pinned.account_id != account_id
                || pinned.organization_id != identity.organization_id
                || pinned.repository_id != identity.repository_id
                || pinned.repository_owner_id != identity.repository_owner_id
            {
                return Err(AccessFailure::PermissionDenied);
            }
        }
        // Actions access path through a FRESH admin connection: the
        // cached connection cannot prove the credential chain still
        // works (F5).
        self.admin
            .force_refresh()
            .await
            .inspect_err(|e| self.invalidate_route_proof(e))
            .map_err(|e| e.to_access_failure())?;
        self.probe_actions_access()
            .await
            .map_err(|e| e.to_access_failure())?;
        Ok(RouteProof {
            checked_at_unix_ms: evidence_started_at,
            valid_until_unix_ms: evidence_started_at.saturating_add(POSITIVE_TTL_MS),
            installation_id,
            account_id,
            organization_id: identity.organization_id,
            repository_id: identity.repository_id,
            repository_owner_id: identity.repository_owner_id,
        })
    }
}

#[cfg(test)]
#[path = "route_proof_tests.rs"]
mod tests;
