//! Per-selector installation discovery, numeric-identity continuity and
//! scoped probes for v2 validation (spec 0011 §4.1 steps 2–3), split
//! from `auth_worker_v2.rs` to keep files within the 400-line budget
//! (AGENTS.md).

use super::predecessor::Predecessor;
use super::step::{probe, Step, Stop};
use shaula_core::auth_context::AccountBinding;
use shaula_core::auth_policy::TargetSelector;
use shaula_core::auth_validation::GitHubValidationPort;
use shaula_core::auth_validation::{
    InstallationLookup, InstallationProof, MetadataReachability, RepoIdentity,
};
use shaula_core::error::{CoreError, ReasonCode};
use shaula_core::github::GitHubTarget;
use shaula_core::ports::Clock;
use shaula_core::registry::{AuthIdentityProof, AuthRepoProof};
use shaula_core::secret::SecretString;
use std::sync::Arc;

/// The inputs of the discovery/probe pass.
pub(super) struct SelectorInput<'a> {
    pub resolver: &'a dyn GitHubValidationPort,
    pub app_id: &'a str,
    pub private_key: &'a SecretString,
    pub clock: &'a Arc<dyn Clock>,
    pub policy: &'a shaula_core::auth_policy::TargetPolicy,
    pub predecessor: &'a Predecessor,
}

/// Proven route bindings plus the identities frozen into the snapshot.
pub(super) type BoundSelectors = (Vec<AccountBinding>, Vec<AuthIdentityProof>);

/// The pass: proven route bindings + identities, or the stop (terminal
/// rejection reason, bounded retry or hard error).
pub(super) async fn discover_and_probe(input: &SelectorInput<'_>) -> Step<BoundSelectors> {
    let mut bindings: Vec<AccountBinding> = Vec::new();
    let mut identities: Vec<AuthIdentityProof> = Vec::new();
    for selector in input.policy.selectors() {
        bind_selector(input, selector, &mut bindings, &mut identities).await?;
        check_account_continuity(input.predecessor, selector, &bindings)?;
        let binding = frozen_binding(&bindings, selector.account())?;
        probe_selector(input, selector, binding).await?;
    }

    // Exact-repository numeric identities (repository id + owner id) are
    // proven and compared with previously proven identities — a
    // same-name rebuild can never rebind (spec 0011 §4.1 step 4, §5.1).
    for selector in input.policy.selectors() {
        prove_repository_identity(input, selector, &bindings, &mut identities).await?;
    }
    Ok((bindings, identities))
}

/// Maps a non-proven installation lookup onto its stop.
fn installation_proof(lookup: InstallationLookup) -> Step<InstallationProof> {
    match lookup {
        InstallationLookup::Proven(proof) => Ok(proof),
        InstallationLookup::NotFound => Err(Stop::Rejected("InstallationNotFound")),
        InstallationLookup::Suspended => Err(Stop::Rejected("InstallationSuspended")),
        InstallationLookup::IdentityMismatch => Err(Stop::Rejected("InstallationChanged")),
        InstallationLookup::PermissionDenied => Err(Stop::Rejected("PermissionDenied")),
        InstallationLookup::Transient { retry_after_ms } => Err(Stop::Retry { retry_after_ms }),
    }
}

/// Discovers the selector's installation and freezes its account binding.
async fn bind_selector(
    input: &SelectorInput<'_>,
    selector: &TargetSelector,
    bindings: &mut Vec<AccountBinding>,
    identities: &mut Vec<AuthIdentityProof>,
) -> Step<()> {
    let proof = installation_proof(
        input
            .resolver
            .installation_for_selector(input.app_id, input.private_key, selector)
            .await,
    )?;
    let binding = AccountBinding {
        account_id: proof.account_id,
        account_kind: proof.account_kind,
        login: proof.login.clone(),
        installation_id: proof.installation_id,
        repository_selection: proof.repository_selection,
        validated_at_ms: input.clock.now_unix_ms(),
    };
    match bindings.iter().find(|b| b.account_id == binding.account_id) {
        Some(existing) => {
            // One account's selectors MUST converge onto one
            // installation; array order never picks a route.
            if existing.installation_id != binding.installation_id {
                return Err(Stop::Rejected("AmbiguousInstallation"));
            }
        }
        None => {
            bindings.push(binding);
            identities.push(AuthIdentityProof {
                login: proof.login,
                account_id: proof.account_id,
                installation_id: proof.installation_id,
                repositories: Vec::new(),
            });
        }
    }
    Ok(())
}

/// Continuity: the SAME login must still be the SAME account id —
/// a reused account name is a DIFFERENT account and can never
/// silently rebind. Installation changes are the EXPLICIT
/// reinstall path and are allowed through this proven Candidate.
fn check_account_continuity(
    predecessor: &Predecessor,
    selector: &TargetSelector,
    bindings: &[AccountBinding],
) -> Step<()> {
    let proof_login = selector.account().to_string();
    if let Some(prior) = predecessor_identities(predecessor)
        .iter()
        .find(|p| p.login.eq_ignore_ascii_case(&proof_login))
    {
        let current = frozen_binding(bindings, &proof_login)?;
        if prior.account_id != current.account_id {
            return Err(Stop::Rejected("InstallationChanged"));
        }
    }
    Ok(())
}

/// The binding frozen for an account; its absence is an internal fault.
fn frozen_binding<'b>(bindings: &'b [AccountBinding], account: &str) -> Step<&'b AccountBinding> {
    bindings
        .iter()
        .find(|b| b.account_matches(account))
        .ok_or_else(|| {
            Stop::Failed(CoreError::new(
                ReasonCode::Internal,
                "selector without frozen binding",
            ))
        })
}

/// The scoped probe for one selector: a real runner access probe for an
/// organization/exact-repository selector, metadata reachability for an
/// account-wide selector.
async fn probe_selector(
    input: &SelectorInput<'_>,
    selector: &TargetSelector,
    binding: &AccountBinding,
) -> Step<()> {
    let target = match selector {
        TargetSelector::Organization { owner } => GitHubTarget::organization(owner.clone())
            .map_err(|e| CoreError::new(ReasonCode::Internal, e.summary))?,
        TargetSelector::Repository { owner, repository } => {
            GitHubTarget::new_repository(owner.clone(), repository.clone())
                .map_err(|e| CoreError::new(ReasonCode::Internal, e.summary))?
        }
        TargetSelector::AccountRepositories { .. } => {
            return match input
                .resolver
                .installation_metadata_reachable(
                    input.app_id,
                    input.private_key,
                    binding.installation_id,
                )
                .await
            {
                MetadataReachability::Reachable => Ok(()),
                MetadataReachability::NotFound => Err(Stop::Rejected("InstallationNotFound")),
                MetadataReachability::PermissionDenied => Err(Stop::Rejected("PermissionDenied")),
                MetadataReachability::Transient { retry_after_ms } => {
                    Err(Stop::Retry { retry_after_ms })
                }
            };
        }
    };
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

/// Proves one exact-repository selector's numeric identity against its
/// binding and the predecessor, recording it on the account's proof.
async fn prove_repository_identity(
    input: &SelectorInput<'_>,
    selector: &TargetSelector,
    bindings: &[AccountBinding],
    identities: &mut [AuthIdentityProof],
) -> Step<()> {
    let TargetSelector::Repository { owner, repository } = selector else {
        return Ok(());
    };
    let Some(binding) = bindings
        .iter()
        .find(|b| b.account_matches(selector.account()))
    else {
        return Ok(());
    };
    match input
        .resolver
        .repository_identity(
            input.app_id,
            input.private_key,
            binding.installation_id,
            owner,
            repository,
        )
        .await
    {
        RepoIdentity::Proven(repo_id, owner_id) => {
            if owner_id != binding.account_id {
                return Err(Stop::Rejected("TargetIdentityChanged"));
            }
            if let Some(prior_repo) = predecessor_identities(input.predecessor)
                .iter()
                .flat_map(|p| p.repositories.iter())
                .find(|r| {
                    r.owner.eq_ignore_ascii_case(owner)
                        && r.repository.eq_ignore_ascii_case(repository)
                })
            {
                if prior_repo.repository_id != repo_id || prior_repo.owner_id != owner_id {
                    return Err(Stop::Rejected("TargetIdentityChanged"));
                }
            }
            if let Some(proof) = identities
                .iter_mut()
                .find(|p| p.login.eq_ignore_ascii_case(selector.account()))
            {
                proof.repositories.push(AuthRepoProof {
                    owner: owner.clone(),
                    repository: repository.clone(),
                    repository_id: repo_id,
                    owner_id,
                });
            }
            Ok(())
        }
        RepoIdentity::Missing => Err(Stop::Rejected("TargetNotAllowed")),
        RepoIdentity::Transient { retry_after_ms } => Err(Stop::Retry { retry_after_ms }),
    }
}

fn predecessor_identities(predecessor: &Predecessor) -> &[AuthIdentityProof] {
    match predecessor {
        Predecessor::V2 { identities, .. } => identities,
        _ => &[],
    }
}
