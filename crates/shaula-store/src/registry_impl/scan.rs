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

        // Structural validation only. Supported candidates remain pending
        // until their own provider's online authentication worker verifies them.
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
            // Never promote offline or apply the GitHub policy schema to a
            // Forgejo target. Unknown kind/version pairs still fail closed.
            let structurally_valid = !candidate.credential_bytes.is_empty()
                && match (candidate.kind.as_str(), candidate.schema_version) {
                    ("github_app", 2) => candidate
                        .policy_json
                        .as_deref()
                        .is_some_and(|p| !p.is_empty()),
                    ("forgejo_token", 1) => candidate.policy_json.as_deref().is_some_and(|json| {
                        serde_json::from_str::<shaula_core::forgejo::ForgejoTarget>(json)
                            .is_ok_and(|target| target.validate().is_ok())
                    }),
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
            // Otherwise remains Validating until the provider worker classifies it.
        }

        report.profiles_retired = self.scan_retirements(now).await?;
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
    pub profiles_retired: usize,
}
