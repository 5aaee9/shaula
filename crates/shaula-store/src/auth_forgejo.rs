//! Forgejo promotion creates Fleet revisions, never an Auth Handoff.

use crate::entities::{
    auth::{github_auth_profile_revisions, github_auth_profiles},
    fleet::{fleet_revisions, fleets},
};
use crate::store::{Store, StoreError, StoreResult};
use sea_orm::{
    ActiveValue::{NotSet, Set},
    ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter,
};
use shaula_core::{
    fleet::{FleetProviderKind, FleetSpec},
    forgejo::{ForgejoScope, ForgejoTarget},
    ports::forgejo::ForgejoAuthProbe,
    registry::AuthPromotion,
};

impl Store {
    pub(super) async fn auth_promote_forgejo(
        &self,
        tx: &DatabaseTransaction,
        candidate: &github_auth_profile_revisions::Model,
        profile: &github_auth_profiles::Model,
        promotion: Option<AuthPromotion>,
        now: i64,
    ) -> StoreResult<()> {
        let target: ForgejoTarget =
            serde_json::from_str(candidate.policy_json.as_deref().unwrap_or_default())
                .map_err(|_| StoreError::Corrupt("Forgejo credential target invalid".into()))?;
        let promotion = promotion.ok_or(StoreError::PolicyDenied {
            reason: "ForgejoScopeUnverified",
        })?;
        let probe = serde_json::from_str::<ForgejoAuthProbe>(&promotion.snapshot_json)
            .map_err(|_| StoreError::Corrupt("Forgejo validation snapshot invalid".into()))?;
        if !shaula_core::forgejo::supports_server_version(&probe.server_version) {
            return Err(StoreError::PolicyDenied {
                reason: "UnsupportedForgejoVersion",
            });
        }
        if probe.checked_at_unix_ms > now
            || probe.valid_until_unix_ms <= now
            || probe.valid_until_unix_ms > probe.checked_at_unix_ms.saturating_add(60_000)
        {
            return Err(StoreError::PolicyDenied {
                reason: "ForgejoValidationExpired",
            });
        }
        let user_scope = target.scope == ForgejoScope::User;
        let identity = |probe: &ForgejoAuthProbe| {
            if user_scope {
                probe.principal_id
            } else {
                probe.target_id
            }
        };
        if target.scope != ForgejoScope::Instance {
            if identity(&probe).is_none_or(|id| id == 0) {
                return Err(StoreError::PolicyDenied {
                    reason: "ForgejoScopeUnverified",
                });
            }
            if let Some(previous) = profile.active_revision {
                let previous = self
                    .auth_revision_get_tx(tx, &candidate.profile_key, previous)
                    .await?
                    .and_then(|row| row.validation_snapshot_json)
                    .and_then(|json| serde_json::from_str::<ForgejoAuthProbe>(&json).ok());
                if previous.as_ref().and_then(identity) != identity(&probe) {
                    return Err(StoreError::PolicyDenied {
                        reason: if user_scope {
                            "ForgejoPrincipalChanged"
                        } else {
                            "ForgejoScopeIdentityChanged"
                        },
                    });
                }
            }
        }
        // All policy rejection occurs BEFORE promotion writes, so the caller
        // can atomically mark a denied candidate Rejected without advancing it.
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
            if spec.kind != FleetProviderKind::Forgejo {
                return Err(StoreError::PolicyDenied {
                    reason: "UnsupportedAuthFormat",
                });
            }
            if spec
                .forgejo
                .as_ref()
                .is_none_or(|section| section.target() != target)
            {
                return Err(StoreError::PolicyDenied {
                    reason: "TargetNotAllowed",
                });
            }
            dependents.push((head, previous));
        }
        let mut active: github_auth_profile_revisions::ActiveModel = candidate.clone().into();
        active.state = Set("Active".into());
        active.reason = Set(None);
        active.validation_snapshot_json =
            Set(Some(serde_json::to_string(&probe).map_err(|_| {
                StoreError::Corrupt("Forgejo validation snapshot invalid".into())
            })?));
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
            next.actor = Set(Some("shaula:forgejo-token-rotation".into()));
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
                    resource_kind: "fleet".into(),
                    action: "auth-rotation".into(),
                    actor: "shaula:forgejo-token-rotation".into(),
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
