//! Forgejo promotion creates Fleet revisions, never an Auth Handoff.

use crate::entities::auth::{github_auth_profile_revisions, github_auth_profiles};
use crate::store::{Store, StoreError, StoreResult};
use sea_orm::DatabaseTransaction;
use shaula_core::{
    fleet::FleetProviderKind,
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
        let dependents = self
            .auth_token_dependents(tx, candidate, |spec| {
                if spec.kind != FleetProviderKind::Forgejo {
                    return Err("UnsupportedAuthFormat");
                }
                if spec
                    .forgejo
                    .as_ref()
                    .is_none_or(|section| section.target() != target)
                {
                    return Err("TargetNotAllowed");
                }
                Ok(())
            })
            .await?;
        let snapshot = serde_json::to_string(&probe)
            .map_err(|_| StoreError::Corrupt("Forgejo validation snapshot invalid".into()))?;
        self.auth_token_commit(
            tx,
            super::token_promotion::TokenPromotion {
                candidate,
                profile,
                snapshot_json: snapshot,
            },
            dependents,
            "shaula:forgejo-token-rotation",
            now,
        )
        .await
    }
}
