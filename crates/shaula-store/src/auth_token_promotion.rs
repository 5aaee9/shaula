//! Shared promotion of Pool token credentials (Forgejo, GitLab): dependent
//! Fleets get a new revision, never an Auth Handoff.

use crate::entities::{
    auth::{github_auth_profile_revisions, github_auth_profiles},
    fleet::{fleet_revisions, fleets},
};
use crate::store::{Store, StoreError, StoreResult};
use sea_orm::{
    ActiveValue::{NotSet, Set},
    ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter,
};
use shaula_core::fleet::FleetSpec;

pub(super) type TokenDependent = (fleets::Model, fleet_revisions::Model);

/// The validated candidate being activated on its profile.
pub(super) struct TokenPromotion<'a> {
    pub candidate: &'a github_auth_profile_revisions::Model,
    pub profile: &'a github_auth_profiles::Model,
    pub snapshot_json: String,
}

impl Store {
    /// Live Fleets whose desired revision uses an older revision of this
    /// profile. `admit` refuses (with a policy reason) any dependent the
    /// candidate's target cannot serve; refusal happens before any write.
    pub(super) async fn auth_token_dependents(
        &self,
        tx: &DatabaseTransaction,
        candidate: &github_auth_profile_revisions::Model,
        admit: impl Fn(&FleetSpec) -> Result<(), &'static str>,
    ) -> StoreResult<Vec<TokenDependent>> {
        let mut dependents = Vec::new();
        for head in fleets::Entity::find()
            .filter(fleets::Column::Tombstone.eq(false))
            .filter(fleets::Column::DeletionMarker.eq(false))
            .all(tx)
            .await?
        {
            let Some(previous) = fleet_revisions::Entity::find()
                .filter(fleet_revisions::Column::FleetKey.eq(&head.key))
                .filter(fleet_revisions::Column::Revision.eq(head.desired_revision))
                .one(tx)
                .await?
            else {
                continue;
            };
            if previous.auth_desired_profile_key != candidate.profile_key
                || previous.auth_desired_revision == candidate.revision
            {
                continue;
            }
            let spec: FleetSpec = serde_json::from_str(&previous.spec_json)
                .map_err(|_| StoreError::Corrupt("dependent Fleet spec invalid".into()))?;
            admit(&spec).map_err(|reason| StoreError::PolicyDenied { reason })?;
            dependents.push((head, previous));
        }
        Ok(dependents)
    }

    /// Activates the candidate with its probe snapshot and moves each
    /// dependent Fleet to a new revision on the promoted credential.
    pub(super) async fn auth_token_commit(
        &self,
        tx: &DatabaseTransaction,
        promotion: TokenPromotion<'_>,
        dependents: Vec<TokenDependent>,
        actor: &str,
        now: i64,
    ) -> StoreResult<()> {
        let TokenPromotion {
            candidate,
            profile,
            snapshot_json,
        } = promotion;
        let mut active: github_auth_profile_revisions::ActiveModel = candidate.clone().into();
        active.state = Set("Active".into());
        active.reason = Set(None);
        active.validation_snapshot_json = Set(Some(snapshot_json));
        github_auth_profile_revisions::Entity::update(active)
            .exec(tx)
            .await?;
        let mut updated: github_auth_profiles::ActiveModel = profile.clone().into();
        updated.active_revision = Set(Some(candidate.revision));
        updated.observed_revision = Set(Some(candidate.revision));
        updated.status = Set("Active".into());
        updated.updated_at = Set(now);
        github_auth_profiles::Entity::update(updated)
            .exec(tx)
            .await?;

        for (head, previous) in dependents {
            let revision = head
                .desired_revision
                .checked_add(1)
                .ok_or_else(|| StoreError::Corrupt("Fleet revision exhausted".into()))?;
            let fence = head
                .mutation_fence
                .checked_add(1)
                .ok_or_else(|| StoreError::Corrupt("Fleet fence exhausted".into()))?;
            let key = head.key.clone();
            let mut next: fleet_revisions::ActiveModel = previous.into();
            next.id = NotSet;
            next.revision = Set(revision);
            next.auth_desired_revision = Set(candidate.revision);
            next.actor = Set(Some(actor.into()));
            next.created_at = Set(now);
            fleet_revisions::Entity::insert(next).exec(tx).await?;
            let mut next: fleets::ActiveModel = head.into();
            next.desired_revision = Set(revision);
            next.mutation_fence = Set(fence);
            next.phase = Set("Reconciling".into());
            next.updated_at = Set(now);
            fleets::Entity::update(next).exec(tx).await?;
            let change = shaula_core::auth::new_attempt_id();
            self.change_insert(tx, &change, &key, revision, "AuthRotation", now)
                .await?;
            self.audit_append(
                tx,
                shaula_core::registry::AuditAppend {
                    authentication: Default::default(),
                    resource_kind: "fleet".into(),
                    action: "auth-rotation".into(),
                    actor: actor.into(),
                    resource_key: key.clone(),
                    revision: Some(revision),
                    outcome: "accepted".into(),
                    detail_json: None,
                    now,
                },
            )
            .await?;
            self.outbox_enqueue(
                tx,
                "fleet",
                "fleet.reconcile",
                &serde_json::json!({"key":key,"revision":revision}).to_string(),
                now,
            )
            .await?;
        }
        Ok(())
    }
}
