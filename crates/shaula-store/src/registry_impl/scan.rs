//! Minimal level-triggered reconcile scan (phase-1 scope).

use shaula_core::error::CoreResult;

use super::core_err;
use super::SqliteControlPlane;

impl SqliteControlPlane {
    /// Minimal level-triggered reconcile scan (phase-1 scope):
    ///
    /// 1. consumes processed outbox markers;
    /// 2. validates and automatically activates current Template Candidates
    ///    from the admitted artifact authority (spec 0017).
    ///
    /// Template publication may converge here; Fleet `observed_revision`
    /// and runtime `Converged` remain owned by the actual Fleet supervisor.
    pub async fn periodic_scan(&self, now: i64) -> CoreResult<ScanReport> {
        let outbox_flushed = self.store.outbox_flush(now).await.map_err(core_err)?;
        let mut report = ScanReport {
            outbox_flushed,
            ..Default::default()
        };

        self.scan_templates(now, &mut report).await?;

        // Auth candidates: structured credential validation only at this
        // stage (kind/identity/allowlist shape). The real github.com
        // access matrix remains a release gate; a passing candidate is
        // promoted with the staged-activation semantics.
        for key in self.store.auth_profiles_list().await.map_err(core_err)? {
            let Some(profile) = self
                .store
                .auth_profile_get(&key.key)
                .await
                .map_err(core_err)?
            else {
                continue;
            };
            if profile.status != "Validating" {
                continue;
            }
            let Some(candidate) = self
                .store
                .auth_revision_get(&key.key, profile.desired_revision)
                .await
                .map_err(core_err)?
            else {
                continue;
            };
            // Structural validation only: a credential that cannot even be
            // parsed is rejected. Structurally valid candidates STAY in
            // Validating — promotion to Active requires real GitHub
            // identity/access validation (spec 0005 section 6, phase 3),
            // never an offline heuristic. Fail closed. A v2 Candidate's
            // authority is its Target policy, not the (empty) legacy
            // allowlist; an unknown schema version never passes.
            let structurally_valid = !candidate.credential_bytes.is_empty()
                && match candidate.schema_version {
                    1 => !candidate.allowlist_json.is_empty(),
                    2 => candidate
                        .policy_json
                        .as_deref()
                        .is_some_and(|p| !p.is_empty()),
                    _ => false,
                };
            if !structurally_valid {
                self.store
                    .auth_scan_apply(
                        &key.key,
                        profile.desired_revision,
                        false,
                        Some("CredentialMalformed"),
                        now,
                    )
                    .await
                    .map_err(core_err)?;
                report.auth_rejected += 1;
            }
            // else: remains Validating until the GitHub validation worker
            // (phase 3) classifies it; no silent promotion.
        }

        Ok(report)
    }
}

/// Outcome counters of one periodic scan pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScanReport {
    pub outbox_flushed: u64,
    /// Candidates passing static validation during this scan.
    pub candidates_ready: usize,
    pub candidates_activated: usize,
    pub candidates_rejected: usize,
    pub auth_promoted: usize,
    pub auth_rejected: usize,
}
