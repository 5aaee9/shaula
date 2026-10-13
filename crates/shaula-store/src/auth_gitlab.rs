//! GitLab token promotion (spec 0043 §7): like Forgejo, dependents get a new
//! Fleet revision; the scope's numeric id must not change across rotations.

use crate::entities::auth::{github_auth_profile_revisions, github_auth_profiles};
use crate::store::{Store, StoreError, StoreResult};
use sea_orm::DatabaseTransaction;
use shaula_core::{
    fleet::FleetProviderKind, gitlab::GitlabTarget, ports::gitlab::GitlabAuthProbe,
    registry::AuthPromotion,
};

impl Store {
    pub(super) async fn auth_promote_gitlab(
        &self,
        tx: &DatabaseTransaction,
        candidate: &github_auth_profile_revisions::Model,
        profile: &github_auth_profiles::Model,
        promotion: Option<AuthPromotion>,
        now: i64,
    ) -> StoreResult<()> {
        let target: GitlabTarget =
            serde_json::from_str(candidate.policy_json.as_deref().unwrap_or_default())
                .map_err(|_| StoreError::Corrupt("GitLab credential target invalid".into()))?;
        let promotion = promotion.ok_or(StoreError::PolicyDenied {
            reason: "GitlabScopeUnverified",
        })?;
        let probe = serde_json::from_str::<GitlabAuthProbe>(&promotion.snapshot_json)
            .map_err(|_| StoreError::Corrupt("GitLab validation snapshot invalid".into()))?;
        if !shaula_core::gitlab::supports_server_version(&probe.server_version) {
            return Err(StoreError::PolicyDenied {
                reason: "UnsupportedGitlabVersion",
            });
        }
        if probe.checked_at_unix_ms > now
            || probe.valid_until_unix_ms <= now
            || probe.valid_until_unix_ms > probe.checked_at_unix_ms.saturating_add(60_000)
        {
            return Err(StoreError::PolicyDenied {
                reason: "GitlabValidationExpired",
            });
        }
        if probe.target_id == 0 {
            return Err(StoreError::PolicyDenied {
                reason: "GitlabScopeUnverified",
            });
        }
        if let Some(previous) = profile.active_revision {
            let previous = self
                .auth_revision_get_tx(tx, &candidate.profile_key, previous)
                .await?
                .and_then(|row| row.validation_snapshot_json)
                .and_then(|json| serde_json::from_str::<GitlabAuthProbe>(&json).ok());
            if previous.map(|previous| previous.target_id) != Some(probe.target_id) {
                return Err(StoreError::PolicyDenied {
                    reason: "GitlabScopeIdentityChanged",
                });
            }
        }
        let dependents = self
            .auth_token_dependents(tx, candidate, |spec| {
                if spec.kind != FleetProviderKind::Gitlab {
                    return Err("UnsupportedAuthFormat");
                }
                if spec
                    .gitlab
                    .as_ref()
                    .is_none_or(|section| section.target() != target)
                {
                    return Err("TargetNotAllowed");
                }
                Ok(())
            })
            .await?;
        let snapshot = serde_json::to_string(&probe)
            .map_err(|_| StoreError::Corrupt("GitLab validation snapshot invalid".into()))?;
        self.auth_token_commit(
            tx,
            super::token_promotion::TokenPromotion {
                candidate,
                profile,
                snapshot_json: snapshot,
            },
            dependents,
            "shaula:gitlab-token-rotation",
            now,
        )
        .await
    }
}
