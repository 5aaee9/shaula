//! v2 Auth Candidate validation (spec 0011 §4.1): App identity proof,
//! per-selector installation discovery with numeric-identity continuity,
//! binding convergence, dynamic metadata reachability, a REAL runner
//! access probe for EVERY organization/exact-repository selector
//! (independent of dependent fleets), and live-dependent route checks —
//! then ONE atomic promotion that freezes bindings + snapshot. Terminal
//! identity failures REJECT the Candidate through an explicit verdict
//! that never falls through into promotion; network, `429`, rate-limit
//! `403` and `5xx` stay Pending with a bounded retry.
//! `WorkerEndpoints` is the transport/configuration seam: production
//! wiring passes the fixed `github.com` hosts; scripted tests pin a
//! local server so the REAL worker flow is exercised end to end.

use crate::auth_worker_predecessor::{load_predecessor, Predecessor};
use crate::auth_worker_probe::{probe_runner_access, ProbeOutcome, WorkerEndpoints};
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::github::GitHubTarget;
use shaula_core::ports::Clock;
use shaula_core::registry::{
    auth_dependent_set_fingerprint, AuthCheckedFleet, AuthPromotion, AuthRevisionRow,
    AuthValidationSnapshot, ControlPlaneStore,
};
use shaula_core::secret::SecretString;
use shaula_scaleset::installation::RepoIdentity;
use shaula_scaleset::AppInstallationResolver;
use std::sync::Arc;

/// Outcome of the whole v2 validation pass — persistence success is
/// deliberately distinct from the validation verdict: a terminal
/// rejection can never be followed by promotion (R2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Verdict {
    /// Candidate promoted.
    Accepted,
    /// Candidate durably rejected; the prior active head is untouched.
    Rejected,
    /// Transient: the Candidate stays Validating with a bounded retry.
    /// Carries the deadline GitHub supplied (Retry-After), if any.
    RetryNeeded { retry_after_ms: Option<i64> },
}

pub(super) async fn validate_v2(
    store: &Arc<dyn ControlPlaneStore>,
    clock: &Arc<dyn Clock>,
    key: &str,
    row: &AuthRevisionRow,
    endpoints: &WorkerEndpoints,
) -> CoreResult<Verdict> {
    let policy = row
        .target_policy()?
        .ok_or_else(|| CoreError::new(ReasonCode::Internal, "v2 revision without target policy"))?;
    let Some(app_id) = row.app_id.clone() else {
        return reject(store, clock, key, row.revision, "CredentialMalformed").await;
    };
    let Some(secret_bytes) = store.auth_credential_bytes(key, row.revision).await? else {
        return Ok(Verdict::RetryNeeded {
            retry_after_ms: None,
        });
    };
    let private_key = SecretString::new(String::from_utf8_lossy(&secret_bytes).into_owned());
    let http = shaula_scaleset::client::production_http_client()
        .map_err(|e| CoreError::new(ReasonCode::Internal, e.summary()))?;
    let resolver = AppInstallationResolver::new(endpoints.api_base.clone(), http, clock.clone());
    // The PREDECESSOR is the continuity authority (F2/R8): its stored
    // App identity must be proven through `/app`, and — for a v2
    // predecessor — its proven numeric identities constrain the
    // Candidate (account ids immutable per login, exact-repo ids
    // immutable across publications, spec 0011 §4.1 step 4, §5.1).
    let predecessor = load_predecessor(store, key).await?;

    // 1. The private key must authenticate as the DECLARED App.
    let verification = match resolver.verify_app(&app_id, &private_key).await {
        Ok(verification) => verification,
        Err(error) => {
            return match error {
                shaula_scaleset::ScalesetError::RateLimited {
                    retry_after_secs, ..
                } => Ok(Verdict::RetryNeeded {
                    retry_after_ms: retry_after_secs.map(|s| s.max(0).saturating_mul(1000)),
                }),
                shaula_scaleset::ScalesetError::Status {
                    status: 401 | 403 | 404,
                    ..
                }
                | shaula_scaleset::ScalesetError::Configuration { .. } => {
                    reject(store, clock, key, row.revision, "Unauthenticated").await
                }
                _ => Ok(Verdict::RetryNeeded {
                    retry_after_ms: None,
                }),
            };
        }
    };
    // Legacy client-ID ↔ numeric-App continuity (spec 0011 §7.5): when
    // the predecessor stored a client-ID string, the `/app` response
    // must carry the SAME client_id — a different App rejects instead of
    // silently rebinding the profile. This check runs for BOTH legacy
    // and v2 predecessors.
    let predecessor_app_id = match &predecessor {
        Predecessor::First => None,
        Predecessor::Legacy { app_id } | Predecessor::V2 { app_id, .. } => Some(app_id),
    };
    if let Some(previous_app_id) = predecessor_app_id {
        if !is_client_id_form(previous_app_id) && previous_app_id != &app_id {
            return reject(store, clock, key, row.revision, "InstallationChanged").await;
        }
        if is_client_id_form(previous_app_id)
            && verification.client_id.as_deref() != Some(previous_app_id)
        {
            return reject(store, clock, key, row.revision, "InstallationChanged").await;
        }
    }

    // 2.+3. Per-selector discovery, continuity and scoped probes —
    // extracted to auth_worker_selectors.rs (F6/F11).
    let (bindings, identities) = match crate::auth_worker_selectors::discover_and_probe(
        &crate::auth_worker_selectors::SelectorInput {
            resolver: &resolver,
            app_id: &app_id,
            private_key: &private_key,
            clock,
            policy: &policy,
            predecessor: &predecessor,
            endpoints,
        },
    )
    .await?
    {
        crate::auth_worker_selectors::SelectorOutcome::Bound(b, i) => (b, i),
        crate::auth_worker_selectors::SelectorOutcome::Rejected(reason) => {
            return reject(store, clock, key, row.revision, reason).await;
        }
        crate::auth_worker_selectors::SelectorOutcome::Retry { retry_after_ms } => {
            return Ok(Verdict::RetryNeeded { retry_after_ms });
        }
    };

    // 4. Live dependent Fleets (§4.1 step 6): policy coverage first
    // (shrink included) over desired AND observed references, then
    // PROOF-ONLY remote validation of every concrete dependent Target —
    // installation access RIGHT NOW, numeric identity continuity, and
    // the real runner probe. The Candidate never mutates fleet
    // authority here.
    let dependents = store.auth_live_dependents(key).await?;
    for dependent in &dependents {
        let Ok(target) = serde_json::from_str::<GitHubTarget>(&dependent.target_json) else {
            return Err(CoreError::new(
                ReasonCode::Internal,
                "dependent fleet target invalid",
            ));
        };
        if !policy.allows(&target) {
            return reject(store, clock, key, row.revision, "TargetPolicyInUse").await;
        }
        let Some(binding) = bindings.iter().find(|b| b.account_matches(target.owner())) else {
            return reject(store, clock, key, row.revision, "TargetNotAllowed").await;
        };
        // Captured with the dependent set: these are retained execution,
        // session and recovery identities, including dynamic-only targets.
        // A newly installed route may change installation across revisions,
        // but it cannot replace any live account/target numeric identity.
        if dependent.retained_contexts.iter().any(|context| {
            context.target != target
                || context.account_id != binding.account_id
                || context.account_kind != binding.account_kind
                || context
                    .organization_id
                    .is_some_and(|id| id != binding.account_id)
        }) {
            return reject(store, clock, key, row.revision, "TargetIdentityChanged").await;
        }
        // Repository Targets must retain installation access and their
        // proven numeric identity at validation time.
        if let GitHubTarget::Repository { owner, repository } = &target {
            match resolver
                .repository_identity(
                    &app_id,
                    &private_key,
                    binding.installation_id,
                    owner,
                    repository,
                )
                .await
            {
                RepoIdentity::Proven(repo_id, owner_id) => {
                    if owner_id != binding.account_id
                        || dependent.retained_contexts.iter().any(|context| {
                            context.repository_id.is_some_and(|id| id != repo_id)
                                || context.repository_owner_id.is_some_and(|id| id != owner_id)
                        })
                    {
                        return reject(store, clock, key, row.revision, "TargetIdentityChanged")
                            .await;
                    }
                    // The Candidate's own exact-repo proofs (step 3)
                    // already agreed with the predecessor; a dependent
                    // repository resolved to a DIFFERENT id than the
                    // selector proof means the repository moved.
                    if let Some(proof) =
                        identities
                            .iter()
                            .flat_map(|p| p.repositories.iter())
                            .find(|r| {
                                r.owner.eq_ignore_ascii_case(owner)
                                    && r.repository.eq_ignore_ascii_case(repository)
                            })
                    {
                        if proof.repository_id != repo_id || proof.owner_id != owner_id {
                            return reject(
                                store,
                                clock,
                                key,
                                row.revision,
                                "TargetIdentityChanged",
                            )
                            .await;
                        }
                    }
                }
                RepoIdentity::Missing => {
                    return reject(store, clock, key, row.revision, "TargetNotAllowed").await;
                }
                RepoIdentity::Transient { retry_after_ms } => {
                    return Ok(Verdict::RetryNeeded { retry_after_ms });
                }
            }
        }
        // The real runner access probe per dependent Target.
        match probe_runner_access(
            &target,
            &app_id,
            binding.installation_id,
            &private_key,
            clock,
            endpoints,
        )
        .await
        {
            ProbeOutcome::Proven => {}
            ProbeOutcome::Terminal(reason) => {
                return reject(store, clock, key, row.revision, reason).await;
            }
            ProbeOutcome::Retry { retry_after_ms } => {
                return Ok(Verdict::RetryNeeded { retry_after_ms });
            }
        }
    }

    // 5. Atomic promotion with the frozen bindings + full snapshot. A
    // dependent-set or fleet identity/fence change RESTAGES.
    let snapshot = AuthValidationSnapshot {
        candidate: (key.to_string(), row.revision),
        dependent_set: auth_dependent_set_fingerprint(&dependents),
        checked_fleets: dependents
            .iter()
            .map(|d| AuthCheckedFleet {
                key: d.fleet_key.clone(),
                incarnation: d.incarnation.clone(),
                revision: d.revision,
                fence: d.fence,
            })
            .collect(),
        identities,
    };
    let snapshot_json = serde_json::to_string(&snapshot)
        .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?;
    let promotion = AuthPromotion {
        bindings,
        snapshot_json,
    };
    match store
        .auth_apply_validation_v2(
            key,
            row.revision,
            true,
            None,
            clock.now_unix_ms(),
            Some(promotion),
        )
        .await?
    {
        shaula_core::registry::AuthPromotionOutcome::Promoted => Ok(Verdict::Accepted),
        _ => Ok(Verdict::RetryNeeded {
            retry_after_ms: None,
        }),
    }
}

fn is_client_id_form(app_id: &str) -> bool {
    !app_id.is_empty() && app_id.bytes().any(|b| !b.is_ascii_digit())
}

/// Durably rejects the Candidate and reports the verdict.
async fn reject(
    store: &Arc<dyn ControlPlaneStore>,
    clock: &Arc<dyn Clock>,
    key: &str,
    revision: i64,
    reason: &str,
) -> CoreResult<Verdict> {
    store
        .auth_apply_validation_v2(
            key,
            revision,
            false,
            Some(reason),
            clock.now_unix_ms(),
            None,
        )
        .await
        .map(|_| Verdict::Rejected)
}

#[cfg(test)]
#[path = "auth_worker_v2_tests.rs"]
pub(crate) mod tests;
