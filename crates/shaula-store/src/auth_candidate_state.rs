//! Shared candidate rejection, preserving the previous active credential.

use crate::entities::auth::{github_auth_profile_revisions, github_auth_profiles};
use crate::store::{Store, StoreResult};
use sea_orm::{ActiveValue::Set, DatabaseTransaction, EntityTrait};

impl Store {
    pub(super) async fn auth_reject_candidate(
        &self,
        tx: &DatabaseTransaction,
        candidate: &github_auth_profile_revisions::Model,
        reason: Option<&str>,
        profile: &github_auth_profiles::Model,
        now: i64,
    ) -> StoreResult<()> {
        let rejected_status = if profile.active_revision.is_some() {
            "Active"
        } else {
            "Rejected"
        };
        let mut candidate_updated: github_auth_profile_revisions::ActiveModel =
            candidate.clone().into();
        candidate_updated.state = Set("Rejected".to_string());
        candidate_updated.reason = Set(reason.map(str::to_string));
        github_auth_profile_revisions::Entity::update(candidate_updated)
            .exec(tx)
            .await?;
        let mut updated: github_auth_profiles::ActiveModel = profile.clone().into();
        updated.status = Set(rejected_status.to_string());
        updated.updated_at = Set(now);
        github_auth_profiles::Entity::update(updated)
            .exec(tx)
            .await?;
        Ok(())
    }
}
