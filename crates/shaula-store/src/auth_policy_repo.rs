//! v2 policy/dependency reads and the desired Resolved Auth Context
//! derivation (spec 0011 §4.2/§5.1), split from `auth_v2_repo` to keep
//! files within the 400-line budget (AGENTS.md).

use sea_orm::{ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter};

use crate::entities::auth::fleet_auth_contexts;
use crate::entities::auth::github_auth_revision_bindings;

use crate::store::{Store, StoreError, StoreResult};
use shaula_core::github::GitHubTarget;

impl Store {
    /// Derives the desired Resolved Auth Context JSON for one fleet
    /// target from the ACTIVE revision read ON the transaction (spec 0011
    /// §4.2 admission). Unsupported revisions cannot establish authority.
    /// `Err` distinguishes structural denial (fleet target
    /// matches no selector) from corruption.
    pub(crate) async fn auth_desired_context_tx(
        &self,
        tx: &DatabaseTransaction,
        profile_key: &str,
        revision: i64,
        spec_json: &str,
    ) -> StoreResult<String> {
        let Some(revision_row) = self.auth_revision_get_tx(tx, profile_key, revision).await? else {
            return Err(StoreError::Corrupt(format!(
                "auth revision {profile_key}/{revision} missing"
            )));
        };
        if revision_row.schema_version != 2 || revision_row.kind != "github_app" {
            return Err(StoreError::PolicyDenied {
                reason: "UnsupportedAuthFormat",
            });
        }
        let policy: shaula_core::auth_policy::TargetPolicy =
            serde_json::from_str(revision_row.policy_json.as_deref().unwrap_or_default())
                .map_err(|e| StoreError::Corrupt(format!("stored v2 policy invalid: {e}")))?;
        let bindings = github_auth_revision_bindings::Entity::find()
            .filter(github_auth_revision_bindings::Column::ProfileKey.eq(profile_key.to_string()))
            .filter(github_auth_revision_bindings::Column::Revision.eq(revision))
            .all(tx)
            .await?
            .iter()
            .filter_map(binding_from_row)
            .collect::<Vec<_>>();
        let target = serde_json::from_str::<serde_json::Value>(spec_json)
            .ok()
            .and_then(|spec| {
                serde_json::from_value::<GitHubTarget>(spec["github"]["target"].clone()).ok()
            })
            .ok_or_else(|| StoreError::Corrupt("fleet spec has no valid github target".into()))?;
        use shaula_core::auth_context::{resolve_desired_context, DesiredContextResolution};
        let app_id = revision_row.app_id.clone().unwrap_or_default();
        match resolve_desired_context(profile_key, revision, &app_id, &policy, &bindings, &target) {
            DesiredContextResolution::Resolved(mut context) => {
                // F6/R5: the ACTIVE revision's proven exact-repository
                // identities carry into FIRST target resolution — a
                // repository recreated between profile activation and its
                // first Fleet cannot silently establish a new pin.
                let json = revision_row
                    .validation_snapshot_json
                    .as_deref()
                    .ok_or_else(|| {
                        StoreError::Corrupt("active v2 validation snapshot missing".into())
                    })?;
                let snapshot: shaula_core::registry::AuthValidationSnapshot =
                    serde_json::from_str(json).map_err(|e| {
                        StoreError::Corrupt(format!("active v2 validation snapshot corrupt: {e}"))
                    })?;
                if snapshot.candidate != (profile_key.to_string(), revision) {
                    return Err(StoreError::Corrupt(
                        "active v2 validation snapshot reference mismatched".into(),
                    ));
                }
                for proof in &snapshot.identities {
                    for repo in &proof.repositories {
                        if repo.owner.eq_ignore_ascii_case(context.target.owner())
                            && context
                                .target
                                .repository_name()
                                .is_some_and(|r| r.eq_ignore_ascii_case(&repo.repository))
                        {
                            if proof.account_id != context.account_id
                                || proof.installation_id != context.installation_id
                                || repo.owner_id != context.account_id
                                || repo.repository_id <= 0
                            {
                                return Err(StoreError::Corrupt(
                                    "exact repository proof contradicts binding".into(),
                                ));
                            }
                            context.repository_id = Some(repo.repository_id);
                            context.repository_owner_id = Some(repo.owner_id);
                        }
                    }
                }
                let exact = policy.selectors().iter().any(|selector| matches!(selector,
                    shaula_core::auth_policy::TargetSelector::Repository { owner, repository }
                    if owner.eq_ignore_ascii_case(context.target.owner()) && context.target.repository_name()
                        .is_some_and(|name| name.eq_ignore_ascii_case(repository))));
                if exact && context.repository_id.is_none() {
                    return Err(StoreError::Corrupt(
                        "exact repository selector has no verified identity".into(),
                    ));
                }
                serde_json::to_string(&context)
                    .map_err(|e| StoreError::Corrupt(format!("context serialization failed: {e}")))
            }
            DesiredContextResolution::NoMatchingSelector => Err(StoreError::PolicyDenied {
                reason: "TargetNotAllowed",
            }),
            DesiredContextResolution::AmbiguousInstallation => Err(StoreError::PolicyDenied {
                reason: "AmbiguousInstallation",
            }),
        }
    }

    pub(crate) async fn auth_bindings_get(
        &self,
        key: &str,
        revision: i64,
    ) -> StoreResult<Vec<shaula_core::auth_context::AccountBinding>> {
        let rows = github_auth_revision_bindings::Entity::find()
            .filter(github_auth_revision_bindings::Column::ProfileKey.eq(key.to_string()))
            .filter(github_auth_revision_bindings::Column::Revision.eq(revision))
            .all(self.connection())
            .await?;
        Ok(rows.iter().filter_map(binding_from_row).collect())
    }

    /// Writes the desired Resolved Auth Context inside the fleet mutation
    /// transaction (spec 0011 §4.2 admission). A structural denial
    /// resolves to `Err(reason)` so the caller can roll the mutation back
    /// as a domain 422 — never silently accepted.
    pub(crate) async fn fleet_auth_context_commit_tx(
        &self,
        tx: &DatabaseTransaction,
        fleet_key: &str,
        profile_key: &str,
        revision: i64,
        spec_json: &str,
        now: i64,
    ) -> StoreResult<Result<(), &'static str>> {
        match self
            .auth_desired_context_tx(tx, profile_key, revision, spec_json)
            .await
        {
            Ok(context_json) => self
                .fleet_auth_context_set_desired_tx(
                    tx,
                    fleet_key,
                    profile_key,
                    revision,
                    &context_json,
                    now,
                )
                .await
                .map(|_| Ok(())),
            Err(StoreError::PolicyDenied { reason }) => Ok(Err(reason)),
            Err(e) => Err(e),
        }
    }
}

pub(crate) fn binding_from_row(
    row: &github_auth_revision_bindings::Model,
) -> Option<shaula_core::auth_context::AccountBinding> {
    let account_kind = match row.account_kind.as_str() {
        "user" => shaula_core::auth_policy::AccountKind::User,
        "organization" => shaula_core::auth_policy::AccountKind::Organization,
        _ => return None,
    };
    let repository_selection = match row.repository_selection.as_str() {
        "all" => shaula_core::auth_context::RepositorySelection::All,
        "selected" => shaula_core::auth_context::RepositorySelection::Selected,
        _ => return None,
    };
    Some(shaula_core::auth_context::AccountBinding {
        account_id: row.account_id,
        account_kind,
        login: row.login.clone(),
        installation_id: row.installation_id,
        repository_selection,
        validated_at_ms: row.validated_at_ms,
    })
}

/// The persisted fleet-context row mapping (entity → port model).
pub(crate) fn fleet_auth_context_row(
    row: fleet_auth_contexts::Model,
) -> shaula_core::registry::FleetAuthContextRow {
    shaula_core::registry::FleetAuthContextRow {
        fleet_key: row.fleet_key,
        desired: row.desired_profile_key.zip(row.desired_revision),
        desired_context_json: row.desired_context_json,
        observed: row.observed_profile_key.zip(row.observed_revision),
        observed_context_json: row.observed_context_json,
        state: row.state,
        reason: row.reason,
    }
}
