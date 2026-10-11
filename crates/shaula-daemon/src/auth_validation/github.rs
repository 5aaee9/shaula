//! v2 Auth Candidate validation (spec 0011 §4.1): App identity proof,
//! per-selector installation discovery with numeric-identity continuity,
//! binding convergence, dynamic metadata reachability, a REAL runner
//! access probe for EVERY organization/exact-repository selector
//! (independent of dependent fleets), and live-dependent route checks —
//! then ONE atomic promotion that freezes bindings + snapshot. Terminal
//! identity failures REJECT the Candidate through an explicit verdict
//! that never falls through into promotion; network, `429`, rate-limit
//! `403` and `5xx` stay Pending with a bounded retry.
//! Concrete GitHub clients are injected through the core validation port;
//! this module owns continuity, rejection, and atomic promotion policy.

use super::predecessor::{load_predecessor, Predecessor};
use super::selectors::{discover_and_probe, SelectorInput};
use super::step::{probe, Step, Stop};
use shaula_core::auth_context::AccountBinding;
use shaula_core::auth_validation::{GitHubValidationPort, RepoIdentity};
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::github::GitHubTarget;
use shaula_core::ports::Clock;
use shaula_core::registry::{
    auth_dependent_set_fingerprint, AuthCheckedFleet, AuthDependentTarget, AuthIdentityProof,
    AuthPromotion, AuthRevisionRow, AuthValidationSnapshot, ControlPlaneStore,
};
use shaula_core::secret::SecretString;
use std::sync::Arc;

/// Outcome of the whole v2 validation pass — persistence success is
/// deliberately distinct from the validation verdict: a terminal
/// rejection can never be followed by promotion (R2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Candidate promoted.
    Accepted,
    /// Candidate durably rejected; the prior active head is untouched.
    Rejected,
    /// Transient: the Candidate stays Validating with a bounded retry.
    /// Carries the deadline GitHub supplied (Retry-After), if any.
    RetryNeeded { retry_after_ms: Option<i64> },
}

/// Everything a fully proven Candidate promotes with.
struct ProvenCandidate {
    bindings: Vec<AccountBinding>,
    identities: Vec<AuthIdentityProof>,
    dependents: Vec<AuthDependentTarget>,
}

pub async fn validate_v2(
    store: &Arc<dyn ControlPlaneStore>,
    clock: &Arc<dyn Clock>,
    key: &str,
    row: &AuthRevisionRow,
    resolver: &dyn GitHubValidationPort,
) -> CoreResult<Verdict> {
    if !row.is_github_app_v2() {
        return Err(CoreError::new(
            ReasonCode::CredentialMalformed,
            "unsupported authentication revision",
        ));
    }
    let policy = row
        .target_policy()?
        .ok_or_else(|| CoreError::internal("v2 revision without target policy"))?;
    let Some(app_id) = row.app_id.clone() else {
        return reject(store, clock, key, row.revision, "CredentialMalformed").await;
    };
    let Some(secret_bytes) = store.auth_credential_bytes(key, row.revision).await? else {
        return Ok(Verdict::RetryNeeded {
            retry_after_ms: None,
        });
    };
    let private_key = SecretString::new(String::from_utf8_lossy(&secret_bytes).into_owned());
    // The PREDECESSOR is the continuity authority (F2/R8): its stored
    // App identity must be proven through `/app`, and — for a v2
    // predecessor — its proven numeric identities constrain the
    // Candidate (account ids immutable per login, exact-repo ids
    // immutable across publications, spec 0011 §4.1 step 4, §5.1).
    let predecessor = load_predecessor(store, key).await?;
    let input = SelectorInput {
        resolver,
        app_id: &app_id,
        private_key: &private_key,
        clock,
        policy: &policy,
        predecessor: &predecessor,
    };
    match prove_candidate(store, key, &input).await {
        Ok(proven) => promote(store, clock, key, row.revision, proven).await,
        Err(Stop::Rejected(reason)) => reject(store, clock, key, row.revision, reason).await,
        Err(Stop::Retry { retry_after_ms }) => Ok(Verdict::RetryNeeded { retry_after_ms }),
        Err(Stop::Failed(error)) => Err(error),
    }
}

/// Every proof the Candidate must pass before promotion, in order.
async fn prove_candidate(
    store: &Arc<dyn ControlPlaneStore>,
    key: &str,
    input: &SelectorInput<'_>,
) -> Step<ProvenCandidate> {
    // 1. The private key must authenticate as the DECLARED App.
    probe(
        input
            .resolver
            .verify_app(input.app_id, input.private_key)
            .await,
    )?;
    check_app_continuity(input.predecessor, input.app_id)?;

    // 2.+3. Per-selector discovery, continuity and scoped probes —
    // extracted to selectors.rs (F6/F11).
    let (bindings, identities) = discover_and_probe(input).await?;

    // 4. Live dependent Fleets (§4.1 step 6): policy coverage first
    // (shrink included) over desired AND observed references, then
    // PROOF-ONLY remote validation of every concrete dependent Target —
    // installation access RIGHT NOW, numeric identity continuity, and
    // the real runner probe. The Candidate never mutates fleet
    // authority here.
    let dependents = store.auth_live_dependents(key).await?;
    for dependent in &dependents {
        check_dependent(input, &bindings, &identities, dependent).await?;
    }
    Ok(ProvenCandidate {
        bindings,
        identities,
        dependents,
    })
}

/// A profile retains its proven numeric App identity across publications.
fn check_app_continuity(predecessor: &Predecessor, app_id: &str) -> Step<()> {
    if let Predecessor::V2 {
        app_id: previous_app_id,
        ..
    } = predecessor
    {
        if previous_app_id != app_id {
            return Err(Stop::Rejected("InstallationChanged"));
        }
    }
    Ok(())
}

/// Captured with the dependent set: these are retained execution,
/// session and recovery identities, including dynamic-only targets.
/// A newly installed route may change installation across revisions,
/// but it cannot replace any live account/target numeric identity.
fn retained_identity_changed(
    dependent: &AuthDependentTarget,
    target: &GitHubTarget,
    binding: &AccountBinding,
) -> bool {
    dependent.retained_contexts.iter().any(|context| {
        context.target != *target
            || context.account_id != binding.account_id
            || context.account_kind != binding.account_kind
            || context
                .organization_id
                .is_some_and(|id| id != binding.account_id)
    })
}

/// Policy coverage, binding, retained identity, repository identity and
/// the real runner access probe for one live dependent Target.
async fn check_dependent(
    input: &SelectorInput<'_>,
    bindings: &[AccountBinding],
    identities: &[AuthIdentityProof],
    dependent: &AuthDependentTarget,
) -> Step<()> {
    let Ok(target) = serde_json::from_str::<GitHubTarget>(&dependent.target_json) else {
        return Err(CoreError::internal("dependent fleet target invalid").into());
    };
    if !input.policy.allows(&target) {
        return Err(Stop::Rejected("TargetPolicyInUse"));
    }
    let Some(binding) = bindings.iter().find(|b| b.account_matches(target.owner())) else {
        return Err(Stop::Rejected("TargetNotAllowed"));
    };
    if retained_identity_changed(dependent, &target, binding) {
        return Err(Stop::Rejected("TargetIdentityChanged"));
    }
    // Repository Targets must retain installation access and their
    // proven numeric identity at validation time.
    if let GitHubTarget::Repository { owner, repository } = &target {
        let identity = input
            .resolver
            .repository_identity(
                input.app_id,
                input.private_key,
                binding.installation_id,
                owner,
                repository,
            )
            .await;
        check_dependent_repository(identity, dependent, binding, identities, owner, repository)?;
    }
    // The real runner access probe per dependent Target.
    probe(
        input
            .resolver
            .probe_runner_access(
                &target,
                input.app_id,
                binding.installation_id,
                input.private_key,
            )
            .await,
    )
}

/// Classifies a dependent repository's resolved identity against its
/// binding, retained contexts and the Candidate's own selector proofs.
fn check_dependent_repository(
    identity: RepoIdentity,
    dependent: &AuthDependentTarget,
    binding: &AccountBinding,
    identities: &[AuthIdentityProof],
    owner: &str,
    repository: &str,
) -> Step<()> {
    match identity {
        RepoIdentity::Proven(repo_id, owner_id) => {
            if owner_id != binding.account_id
                || dependent.retained_contexts.iter().any(|context| {
                    context.repository_id.is_some_and(|id| id != repo_id)
                        || context.repository_owner_id.is_some_and(|id| id != owner_id)
                })
            {
                return Err(Stop::Rejected("TargetIdentityChanged"));
            }
            // The Candidate's own exact-repo proofs (step 3)
            // already agreed with the predecessor; a dependent
            // repository resolved to a DIFFERENT id than the
            // selector proof means the repository moved.
            if let Some(proof) = identities
                .iter()
                .flat_map(|p| p.repositories.iter())
                .find(|r| {
                    r.owner.eq_ignore_ascii_case(owner)
                        && r.repository.eq_ignore_ascii_case(repository)
                })
            {
                if proof.repository_id != repo_id || proof.owner_id != owner_id {
                    return Err(Stop::Rejected("TargetIdentityChanged"));
                }
            }
            Ok(())
        }
        RepoIdentity::Missing => Err(Stop::Rejected("TargetNotAllowed")),
        RepoIdentity::Transient { retry_after_ms } => Err(Stop::Retry { retry_after_ms }),
    }
}

/// Step 5: atomic promotion with the frozen bindings + full snapshot. A
/// dependent-set or fleet identity/fence change RESTAGES.
async fn promote(
    store: &Arc<dyn ControlPlaneStore>,
    clock: &Arc<dyn Clock>,
    key: &str,
    revision: i64,
    proven: ProvenCandidate,
) -> CoreResult<Verdict> {
    let ProvenCandidate {
        bindings,
        identities,
        dependents,
    } = proven;
    let snapshot = AuthValidationSnapshot {
        candidate: (key.to_string(), revision),
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
    let snapshot_json =
        serde_json::to_string(&snapshot).map_err(|e| CoreError::internal(e.to_string()))?;
    let promotion = AuthPromotion {
        bindings,
        snapshot_json,
    };
    match store
        .auth_apply_validation_v2(
            key,
            revision,
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
