//! v2 Auth promotion (spec 0011 §4.1 step 7, §5.1): the Candidate
//! becomes Active only inside ONE transaction that re-checks the desired
//! head, the dependent-set coverage and fingerprint, freezes the verified
//! Account Bindings and retargets live fleets. Partial publication is
//! impossible: every gate and the head advance share the transaction.

use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, QueryOrder};

use crate::entities::auth::{
    github_auth_profile_revisions, github_auth_profiles, github_auth_revision_bindings,
};
use crate::entities::fleet::{fleet_auth_handoffs, fleet_revisions, fleets};
use crate::store::{Store, StoreError, StoreResult};
use shaula_core::github::GitHubTarget;
use shaula_core::registry::{AuthPromotion, AuthPromotionOutcome};

enum ValidationResult<'a> {
    Accepted(Option<AuthPromotion>),
    Rejected(Option<&'a str>),
}

impl Store {
    /// Staged activation requires the validator's frozen bindings and snapshot.
    pub(crate) async fn auth_apply_full(
        &self,
        key: &str,
        revision: i64,
        accepted: bool,
        reason: Option<&str>,
        now: i64,
        promotion: Option<AuthPromotion>,
    ) -> StoreResult<AuthPromotionOutcome> {
        let validation = if accepted {
            ValidationResult::Accepted(promotion)
        } else {
            ValidationResult::Rejected(reason)
        };
        let tx = self.begin().await?;
        let result = self
            .auth_apply_full_tx(&tx, key, revision, now, validation)
            .await;
        match result {
            Ok(outcome) => tx.commit().await.map(|_| outcome).map_err(StoreError::from),
            Err(e) => {
                let _ = tx.rollback().await;
                Err(e)
            }
        }
    }

    async fn auth_apply_full_tx(
        &self,
        tx: &DatabaseTransaction,
        key: &str,
        revision: i64,
        now: i64,
        validation: ValidationResult<'_>,
    ) -> StoreResult<AuthPromotionOutcome> {
        // R10-07: BOTH reads happen ON THE TRANSACTION so a concurrent PUT
        // that advances the desired head can never be clobbered.
        let candidate = self
            .auth_revision_get_tx(tx, key, revision)
            .await?
            .ok_or_else(|| {
                StoreError::Corrupt(format!("auth revision {key}/{revision} missing"))
            })?;
        let profile = github_auth_profiles::Entity::find_by_id(key.to_string())
            .one(tx)
            .await?
            .ok_or_else(|| StoreError::Corrupt(format!("auth profile {key} missing")))?;
        if profile.deletion_requested {
            return Err(StoreError::Conflict {
                resource: format!("{key} is retiring"),
            });
        }
        if profile.desired_revision != revision {
            return Err(StoreError::Conflict {
                resource: format!("{key}/{revision} is no longer desired"),
            });
        }
        // Both successful and failed callbacks require a still-validating
        // Candidate. A late failure must not rewrite an Active revision.
        if candidate.state != "Validating" {
            return Err(StoreError::Conflict {
                resource: format!("{key}/{revision} is not a validating candidate"),
            });
        }

        let (outcome, reason) = match validation {
            ValidationResult::Rejected(reason) => {
                self.auth_reject_candidate(tx, &candidate, reason, &profile, now)
                    .await?;
                (AuthPromotionOutcome::Rejected, reason)
            }
            ValidationResult::Accepted(promotion)
                if candidate.schema_version == 2 && candidate.kind == "github_app" =>
            {
                match self
                    .auth_promote_v2(tx, &candidate, &profile, promotion, now)
                    .await?
                {
                    V2Promotion::Promoted => (AuthPromotionOutcome::Promoted, None),
                    V2Promotion::Rejected(reason) => {
                        self.auth_reject_candidate(tx, &candidate, Some(reason), &profile, now)
                            .await?;
                        (AuthPromotionOutcome::Rejected, Some(reason))
                    }
                    V2Promotion::Restaged => (AuthPromotionOutcome::Restaged, None),
                }
            }
            ValidationResult::Accepted(promotion)
                if candidate.schema_version == 1 && candidate.kind == "forgejo_token" =>
            {
                match self
                    .auth_promote_forgejo(tx, &candidate, &profile, promotion, now)
                    .await
                {
                    Ok(()) => (AuthPromotionOutcome::Promoted, None),
                    Err(StoreError::PolicyDenied { reason }) => {
                        self.auth_reject_candidate(tx, &candidate, Some(reason), &profile, now)
                            .await?;
                        (AuthPromotionOutcome::Rejected, Some(reason))
                    }
                    Err(error) => return Err(error),
                }
            }
            ValidationResult::Accepted(_) => {
                return Err(StoreError::PolicyDenied {
                    reason: "UnsupportedAuthFormat",
                });
            }
        };

        use crate::entities::template::profile_changes;
        profile_changes::Entity::update_many()
            .col_expr(
                profile_changes::Column::State,
                sea_orm::sea_query::Expr::value(match outcome {
                    AuthPromotionOutcome::Promoted => "Converged",
                    AuthPromotionOutcome::Restaged => "Pending",
                    AuthPromotionOutcome::Rejected => "Rejected",
                }),
            )
            .col_expr(
                profile_changes::Column::Reason,
                sea_orm::sea_query::Expr::value(reason.map(str::to_string)),
            )
            .col_expr(
                profile_changes::Column::UpdatedAt,
                sea_orm::sea_query::Expr::value(now),
            )
            .filter(profile_changes::Column::ResourceKind.eq(
                if candidate.kind == "forgejo_token" {
                    "forgejo_auth_profile"
                } else {
                    "github_auth_profile"
                },
            ))
            .filter(profile_changes::Column::ProfileKey.eq(key))
            .filter(profile_changes::Column::Revision.eq(revision))
            .exec(tx)
            .await?;
        Ok(outcome)
    }

    /// v2 promotion gates (spec 0011 §5.1), all in the caller's
    /// transaction: policy coverage of every live dependent Target, the
    /// dependent-set fingerprint CAS, then binding freeze + head advance.
    async fn auth_promote_v2(
        &self,
        tx: &DatabaseTransaction,
        candidate: &github_auth_profile_revisions::Model,
        profile: &github_auth_profiles::Model,
        promotion: Option<AuthPromotion>,
        now: i64,
    ) -> StoreResult<V2Promotion> {
        let Some(promotion) = promotion else {
            return Err(StoreError::Corrupt(
                "v2 promotion requires validated bindings and snapshot".into(),
            ));
        };
        let policy_json = candidate.policy_json.as_deref().unwrap_or_default();
        let policy: shaula_core::auth_policy::TargetPolicy = serde_json::from_str(policy_json)
            .map_err(|e| StoreError::Corrupt(format!("stored v2 policy invalid: {e}")))?;
        let dependents = self
            .auth_live_dependents_tx(tx, &candidate.profile_key)
            .await?;
        for dependent in &dependents {
            let Ok(target) = serde_json::from_str::<GitHubTarget>(&dependent.target_json) else {
                return Err(StoreError::Corrupt(format!(
                    "fleet {} target invalid",
                    dependent.fleet_key
                )));
            };
            if !policy.allows(&target) {
                // Shrink gate: a policy that strands a live Fleet Target
                // can never activate (spec 0011 §5.1).
                return Ok(V2Promotion::Rejected("TargetPolicyInUse"));
            }
        }
        let snapshot: shaula_core::registry::AuthValidationSnapshot =
            serde_json::from_str(&promotion.snapshot_json)
                .map_err(|e| StoreError::Corrupt(format!("validation snapshot invalid: {e}")))?;
        if snapshot.candidate != (candidate.profile_key.clone(), candidate.revision) {
            return Ok(V2Promotion::Restaged);
        }
        if snapshot.dependent_set
            != shaula_core::registry::auth_dependent_set_fingerprint(&dependents)
        {
            // A Fleet referencing this profile appeared or changed during
            // validation: back to validation, never promote stale checks.
            return Ok(V2Promotion::Restaged);
        }
        // The FULL checked state must still hold: fleet identity,
        // revision AND mutation fence. Any concurrent fleet mutation
        // between validation and promotion restages the Candidate.
        if dependents.iter().any(|dependent| {
            !snapshot.checked_fleets.iter().any(|checked| {
                checked.key == dependent.fleet_key
                    && checked.incarnation == dependent.incarnation
                    && checked.revision == dependent.revision
                    && checked.fence == dependent.fence
            })
        }) {
            return Ok(V2Promotion::Restaged);
        }
        for checked in &snapshot.checked_fleets {
            let Some(fleet) = fleets::Entity::find_by_id(checked.key.clone())
                .one(tx)
                .await?
            else {
                return Ok(V2Promotion::Restaged);
            };
            let drifted = fleet.incarnation != checked.incarnation
                || fleet.mutation_fence != checked.fence
                || dependents.iter().any(|d| {
                    d.fleet_key == checked.key
                        && (d.revision != checked.revision
                            || d.incarnation != checked.incarnation
                            || d.fence != checked.fence)
                });
            if drifted {
                return Ok(V2Promotion::Restaged);
            }
        }

        self.auth_bindings_replace_tx(
            tx,
            &candidate.profile_key,
            candidate.revision,
            &promotion.bindings,
        )
        .await?;
        let mut candidate_updated: github_auth_profile_revisions::ActiveModel =
            candidate.clone().into();
        candidate_updated.state = Set("Active".to_string());
        candidate_updated.reason = Set(None);
        candidate_updated.validation_snapshot_json = Set(Some(promotion.snapshot_json));
        github_auth_profile_revisions::Entity::update(candidate_updated)
            .exec(tx)
            .await?;
        self.auth_advance_head(tx, candidate, profile, now).await?;
        Ok(V2Promotion::Promoted)
    }

    /// The shared head advance: active/observed revision, profile status
    /// and the handoff retarget of every live dependent fleet.
    async fn auth_advance_head(
        &self,
        tx: &DatabaseTransaction,
        candidate: &github_auth_profile_revisions::Model,
        profile: &github_auth_profiles::Model,
        now: i64,
    ) -> StoreResult<()> {
        let mut updated: github_auth_profiles::ActiveModel = profile.clone().into();
        updated.active_revision = Set(Some(candidate.revision));
        updated.observed_revision = Set(Some(candidate.revision));
        updated.status = Set("Active".to_string());
        updated.updated_at = Set(now);
        github_auth_profiles::Entity::update(updated)
            .exec(tx)
            .await?;
        self.retarget_fleets_on_rotation(tx, &candidate.profile_key, candidate.revision, now)
            .await
    }

    /// Points the handoff desired tuple of every fleet CURRENTLY depending
    /// on this auth profile at the newly active revision, and advances the
    /// fleet's desired Resolved Auth Context to match (spec 0011 §5.2):
    /// the handoff ref tuple alone never moves without its context
    /// intent. Update-only: never resurrects a removed handoff. Fleets
    /// that only OBSERVE this profile (cross-profile handoff in flight)
    /// keep the NEW profile's desired context untouched.
    async fn retarget_fleets_on_rotation(
        &self,
        tx: &DatabaseTransaction,
        key: &str,
        active_revision: i64,
        now: i64,
    ) -> StoreResult<()> {
        let depending = fleet_auth_handoffs::Entity::find()
            .filter(fleet_auth_handoffs::Column::DesiredProfileKey.eq(key.to_string()))
            .all(tx)
            .await?;
        for handoff in depending {
            let fleet_key = handoff.fleet_key.clone();
            self.handoff_retarget_desired(tx, &fleet_key, key, active_revision)
                .await?;
            // The desired context must track the retargeted revision, or
            // the v2 handoff would be missing its resolution intent.
            let Some(latest) = fleet_revisions::Entity::find()
                .filter(fleet_revisions::Column::FleetKey.eq(&fleet_key))
                .order_by_desc(fleet_revisions::Column::Revision)
                .one(tx)
                .await?
            else {
                continue;
            };
            let is_forgejo =
                serde_json::from_str::<shaula_core::fleet::FleetSpec>(&latest.spec_json)
                    .map(|spec| spec.kind == shaula_core::fleet::FleetProviderKind::Forgejo)
                    .unwrap_or(false);
            if !is_forgejo {
                self.fleet_auth_context_commit_tx(
                    tx,
                    &fleet_key,
                    key,
                    active_revision,
                    &latest.spec_json,
                    now,
                )
                .await?
                .map_err(|reason| StoreError::PolicyDenied { reason })?;
            }
        }
        Ok(())
    }

    /// Freezes the verified Account Bindings of a revision (replace
    /// semantics make a retry of the same promotion idempotent).
    async fn auth_bindings_replace_tx(
        &self,
        tx: &DatabaseTransaction,
        key: &str,
        revision: i64,
        bindings: &[shaula_core::auth_context::AccountBinding],
    ) -> StoreResult<()> {
        github_auth_revision_bindings::Entity::delete_many()
            .filter(github_auth_revision_bindings::Column::ProfileKey.eq(key.to_string()))
            .filter(github_auth_revision_bindings::Column::Revision.eq(revision))
            .exec(tx)
            .await?;
        for binding in bindings {
            let row = github_auth_revision_bindings::ActiveModel {
                id: Default::default(),
                profile_key: Set(key.to_string()),
                revision: Set(revision),
                account_id: Set(binding.account_id),
                account_kind: Set(binding.account_kind.as_str().to_string()),
                login: Set(binding.login.clone()),
                installation_id: Set(binding.installation_id),
                repository_selection: Set(binding.repository_selection.as_str().to_string()),
                validated_at_ms: Set(binding.validated_at_ms),
            };
            github_auth_revision_bindings::Entity::insert(row)
                .exec(tx)
                .await?;
        }
        Ok(())
    }
}

#[path = "auth_candidate_state.rs"]
mod candidate_state;
#[path = "auth_forgejo.rs"]
mod forgejo;

/// Classification of one v2 promotion attempt inside the transaction.
enum V2Promotion {
    Promoted,
    Rejected(&'static str),
    Restaged,
}
