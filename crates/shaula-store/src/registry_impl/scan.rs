//! Minimal level-triggered reconcile scan (phase-1 scope).

use shaula_core::error::CoreResult;
use shaula_core::registry::ControlPlaneStore;

use super::core_err;
use super::SqliteControlPlane;

impl SqliteControlPlane {
    /// Minimal level-triggered reconcile scan (phase-1 scope):
    ///
    /// 1. consumes processed outbox markers;
    /// 2. runs static validation over `Validating` Template Candidates —
    ///    the admitted artifact manifest is the sole authority — moving
    ///    accepted candidates to `Ready`. Activation still requires an
    ///    exact conformance attestation.
    ///
    /// The scan never advances `observed_revision` or claims `Converged`:
    /// those facts are only written by a supervisor that has actually
    /// classified the revision (spec 0002: MUST NOT misreport Converged).
    pub async fn periodic_scan(&self, now: i64) -> CoreResult<ScanReport> {
        let outbox_flushed = self.store.outbox_flush(now).await.map_err(core_err)?;
        let mut report = ScanReport {
            outbox_flushed,
            ..Default::default()
        };

        for profile in self
            .store
            .template_profiles_list()
            .await
            .map_err(core_err)?
        {
            if profile.deletion_requested {
                continue;
            }
            let candidate = self
                .store
                .template_revision_get(&profile.key, profile.desired_revision)
                .await
                .map_err(core_err)?;
            let Some(candidate) = candidate else {
                continue;
            };
            if candidate.state != "Validating" {
                continue;
            }
            // Static validation: re-open the published artifact by digest
            // and verify manifest + shape. No infrastructure mutation.
            let manifest_yaml =
                match ControlPlaneStore::artifact_manifest(self, &candidate.artifact_digest).await?
                {
                    Some(manifest) => manifest,
                    None => {
                        self.store
                            .template_revision_validated(
                                shaula_core::template::TemplateValidationRecord {
                                    key: profile.key.clone(),
                                    revision: profile.desired_revision,
                                    ready: false,
                                    platform: candidate
                                        .platform
                                        .clone()
                                        .unwrap_or_else(|| "other".into()),
                                    bindings_contract: candidate
                                        .bindings_contract
                                        .clone()
                                        .unwrap_or_else(|| "unknown".into()),
                                    manifest_json: String::new(),
                                    lock_digest: String::new(),
                                    reason: Some("artifact digest is not published".into()),
                                },
                            )
                            .await
                            .map_err(core_err)?;
                        report.candidates_rejected += 1;
                        continue;
                    }
                };
            let validated = shaula_template_manifest::parse_and_validate(&manifest_yaml);
            match validated {
                Ok(manifest) => {
                    self.store
                        .template_revision_validated(
                            shaula_core::template::TemplateValidationRecord {
                                key: profile.key.clone(),
                                revision: profile.desired_revision,
                                ready: true,
                                platform: manifest.platform.clone(),
                                bindings_contract: manifest.bindings_contract.clone(),
                                manifest_json: manifest_yaml.clone(),
                                lock_digest: String::new(),
                                reason: None,
                            },
                        )
                        .await
                        .map_err(core_err)?;
                    // Keep the profile status head in step with the
                    // revision state so reads observe a single truth.
                    self.store
                        .template_profile_set_status(&profile.key, "Ready", now)
                        .await
                        .map_err(core_err)?;
                    report.candidates_ready += 1;
                }
                Err(reason) => {
                    self.store
                        .template_revision_validated(
                            shaula_core::template::TemplateValidationRecord {
                                key: profile.key.clone(),
                                revision: profile.desired_revision,
                                ready: false,
                                platform: "other".into(),
                                bindings_contract: "unknown".into(),
                                manifest_json: manifest_yaml.clone(),
                                lock_digest: String::new(),
                                reason: Some(reason.clone()),
                            },
                        )
                        .await
                        .map_err(core_err)?;
                    report.candidates_rejected += 1;
                }
            }
        }

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
    pub candidates_ready: usize,
    pub candidates_rejected: usize,
    pub auth_promoted: usize,
    pub auth_rejected: usize,
}

/// Thin re-export wrapper so the scan can validate manifests without
/// importing the whole template runtime.
mod shaula_template_manifest {
    pub struct ValidManifest {
        pub platform: String,
        pub bindings_contract: String,
    }

    pub fn parse_and_validate(yaml: &str) -> Result<ValidManifest, String> {
        let manifest: shaula_core::template::ProfileManifest =
            serde_yaml::from_str(yaml).map_err(|e| format!("manifest invalid: {e}"))?;
        manifest.validate().map_err(|e| e.summary)?;
        Ok(ValidManifest {
            platform: manifest.platform,
            bindings_contract: manifest.bindings_contract,
        })
    }
}
