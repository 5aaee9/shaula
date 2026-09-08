//! Per-selector installation discovery, numeric-identity continuity and
//! scoped probes for v2 validation (spec 0011 §4.1 steps 2–3), split
//! from `auth_worker_v2.rs` to keep files within the 400-line budget
//! (AGENTS.md).

use crate::auth_worker_predecessor::Predecessor;
use crate::auth_worker_probe::WorkerEndpoints;
use shaula_core::auth_context::AccountBinding;
use shaula_core::auth_policy::TargetSelector;
use shaula_core::error::CoreResult;
use shaula_core::github::GitHubTarget;
use shaula_core::ports::Clock;
use shaula_core::registry::{AuthIdentityProof, AuthRepoProof};
use shaula_core::secret::SecretString;
use shaula_scaleset::installation::{InstallationLookup, MetadataReachability, RepoIdentity};
use shaula_scaleset::AppInstallationResolver;
use std::sync::Arc;

/// The inputs of the discovery/probe pass.
pub(super) struct SelectorInput<'a> {
    pub resolver: &'a AppInstallationResolver,
    pub app_id: &'a str,
    pub private_key: &'a SecretString,
    pub clock: &'a Arc<dyn Clock>,
    pub policy: &'a shaula_core::auth_policy::TargetPolicy,
    pub predecessor: &'a Predecessor,
    pub endpoints: &'a WorkerEndpoints,
}

/// The pass outcome: proven route bindings + identities, a terminal
/// rejection reason, or a bounded retry.
pub(super) enum SelectorOutcome {
    Bound(Vec<AccountBinding>, Vec<AuthIdentityProof>),
    Rejected(&'static str),
    Retry { retry_after_ms: Option<i64> },
}

pub(super) async fn discover_and_probe(input: &SelectorInput<'_>) -> CoreResult<SelectorOutcome> {
    let SelectorInput {
        resolver,
        app_id,
        private_key,
        clock,
        policy,
        predecessor,
        endpoints,
    } = input;
    let mut bindings: Vec<AccountBinding> = Vec::new();
    let mut identities: Vec<AuthIdentityProof> = Vec::new();
    for selector in policy.selectors() {
        match resolver
            .installation_for_selector(app_id, private_key, selector)
            .await
        {
            InstallationLookup::Proven(proof) => {
                let binding = AccountBinding {
                    account_id: proof.account_id,
                    account_kind: proof.account_kind,
                    login: proof.login.clone(),
                    installation_id: proof.installation_id,
                    repository_selection: proof.repository_selection,
                    validated_at_ms: clock.now_unix_ms(),
                };
                match bindings.iter().find(|b| b.account_id == binding.account_id) {
                    Some(existing) => {
                        // One account's selectors MUST converge onto one
                        // installation; array order never picks a route.
                        if existing.installation_id != binding.installation_id {
                            return Ok(SelectorOutcome::Rejected("AmbiguousInstallation"));
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
            }
            InstallationLookup::NotFound => {
                return Ok(SelectorOutcome::Rejected("InstallationNotFound"))
            }
            InstallationLookup::Suspended => {
                return Ok(SelectorOutcome::Rejected("InstallationSuspended"))
            }
            InstallationLookup::IdentityMismatch => {
                return Ok(SelectorOutcome::Rejected("InstallationChanged"))
            }
            InstallationLookup::PermissionDenied => {
                return Ok(SelectorOutcome::Rejected("PermissionDenied"))
            }
            InstallationLookup::Transient { retry_after_ms } => {
                return Ok(SelectorOutcome::Retry { retry_after_ms })
            }
        }
        // Continuity: the SAME login must still be the SAME account id —
        // a reused account name is a DIFFERENT account and can never
        // silently rebind. Installation changes are the EXPLICIT
        // reinstall path and are allowed through this proven Candidate.
        let proof_login = selector.account().to_string();
        if let Some(prior) = predecessor_identities(predecessor)
            .iter()
            .find(|p| p.login.eq_ignore_ascii_case(&proof_login))
        {
            let Some(current) = bindings.iter().find(|b| b.account_matches(&proof_login)) else {
                return Err(shaula_core::error::CoreError::new(
                    shaula_core::error::ReasonCode::Internal,
                    "selector without frozen binding",
                ));
            };
            if prior.account_id != current.account_id {
                return Ok(SelectorOutcome::Rejected("InstallationChanged"));
            }
        }

        let Some(binding) = bindings
            .iter()
            .find(|b| b.account_matches(selector.account()))
        else {
            return Err(shaula_core::error::CoreError::new(
                shaula_core::error::ReasonCode::Internal,
                "selector without frozen binding",
            ));
        };
        match selector {
            TargetSelector::Organization { owner } => {
                let target = GitHubTarget::organization(owner.clone()).map_err(|e| {
                    shaula_core::error::CoreError::new(
                        shaula_core::error::ReasonCode::Internal,
                        e.summary,
                    )
                })?;
                match crate::auth_worker_probe::probe_runner_access(
                    &target,
                    app_id,
                    binding.installation_id,
                    private_key,
                    clock,
                    endpoints,
                )
                .await
                {
                    crate::auth_worker_probe::ProbeOutcome::Proven => {}
                    crate::auth_worker_probe::ProbeOutcome::Terminal(reason) => {
                        return Ok(SelectorOutcome::Rejected(reason));
                    }
                    crate::auth_worker_probe::ProbeOutcome::Retry { retry_after_ms } => {
                        return Ok(SelectorOutcome::Retry { retry_after_ms });
                    }
                }
            }
            TargetSelector::Repository { owner, repository } => {
                let target = GitHubTarget::new_repository(owner.clone(), repository.clone())
                    .map_err(|e| {
                        shaula_core::error::CoreError::new(
                            shaula_core::error::ReasonCode::Internal,
                            e.summary,
                        )
                    })?;
                match crate::auth_worker_probe::probe_runner_access(
                    &target,
                    app_id,
                    binding.installation_id,
                    private_key,
                    clock,
                    endpoints,
                )
                .await
                {
                    crate::auth_worker_probe::ProbeOutcome::Proven => {}
                    crate::auth_worker_probe::ProbeOutcome::Terminal(reason) => {
                        return Ok(SelectorOutcome::Rejected(reason));
                    }
                    crate::auth_worker_probe::ProbeOutcome::Retry { retry_after_ms } => {
                        return Ok(SelectorOutcome::Retry { retry_after_ms });
                    }
                }
            }
            TargetSelector::AccountRepositories { .. } => {
                match resolver
                    .installation_metadata_reachable(app_id, private_key, binding.installation_id)
                    .await
                {
                    MetadataReachability::Reachable => {}
                    MetadataReachability::NotFound => {
                        return Ok(SelectorOutcome::Rejected("InstallationNotFound"))
                    }
                    MetadataReachability::PermissionDenied => {
                        return Ok(SelectorOutcome::Rejected("PermissionDenied"))
                    }
                    MetadataReachability::Transient { retry_after_ms } => {
                        return Ok(SelectorOutcome::Retry { retry_after_ms });
                    }
                }
            }
        }
    }

    // Exact-repository numeric identities (repository id + owner id) are
    // proven and compared with previously proven identities — a
    // same-name rebuild can never rebind (spec 0011 §4.1 step 4, §5.1).
    for selector in policy.selectors() {
        let TargetSelector::Repository { owner, repository } = selector else {
            continue;
        };
        let Some(binding) = bindings
            .iter()
            .find(|b| b.account_matches(selector.account()))
        else {
            continue;
        };
        match resolver
            .repository_identity(
                app_id,
                private_key,
                binding.installation_id,
                owner,
                repository,
            )
            .await
        {
            RepoIdentity::Proven(repo_id, owner_id) => {
                if owner_id != binding.account_id {
                    return Ok(SelectorOutcome::Rejected("TargetIdentityChanged"));
                }
                if let Some(prior_repo) = predecessor_identities(predecessor)
                    .iter()
                    .flat_map(|p| p.repositories.iter())
                    .find(|r| {
                        r.owner.eq_ignore_ascii_case(owner)
                            && r.repository.eq_ignore_ascii_case(repository)
                    })
                {
                    if prior_repo.repository_id != repo_id || prior_repo.owner_id != owner_id {
                        return Ok(SelectorOutcome::Rejected("TargetIdentityChanged"));
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
            }
            RepoIdentity::Missing => return Ok(SelectorOutcome::Rejected("TargetNotAllowed")),
            RepoIdentity::Transient { retry_after_ms } => {
                return Ok(SelectorOutcome::Retry { retry_after_ms });
            }
        }
    }
    Ok(SelectorOutcome::Bound(bindings, identities))
}

fn predecessor_identities(predecessor: &Predecessor) -> &[AuthIdentityProof] {
    match predecessor {
        Predecessor::V2 { identities, .. } => identities,
        _ => &[],
    }
}
