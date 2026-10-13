//! One captured authority check for claim ingest and exact job reads.
use super::{decode, encode, rows};
use crate::{StoreError, StoreResult};
use sea_orm::DatabaseTransaction;
use sha2::{Digest, Sha256};
use shaula_core::{
    fleet::{FleetProviderKind, FleetSpec},
    gitlab::GitlabTarget,
    registry::FleetRuntimeGuard,
};

pub(super) struct ScopeSnapshot {
    pub key: String,
    pub target: GitlabTarget,
}

/// The scope key isolates Fleet incarnation, instance URL, scope and Auth Profile.
pub(super) async fn current(
    tx: &DatabaseTransaction,
    fleet: &str,
    guard: &FleetRuntimeGuard,
) -> StoreResult<Option<ScopeSnapshot>> {
    let found = rows(
        tx,
        "SELECT r.spec_json FROM fleets f JOIN fleet_revisions r
         ON r.fleet_key=f.key AND r.incarnation=f.incarnation AND r.revision=f.desired_revision
         WHERE f.key=? AND f.incarnation=? AND f.desired_revision=? AND f.mutation_fence=?
         AND f.deletion_marker=0 AND f.tombstone=0",
        vec![
            fleet.into(),
            guard.incarnation.clone().into(),
            guard.desired_revision.into(),
            guard.mutation_fence.into(),
        ],
    )
    .await?;
    let Some(row) = found.first() else {
        return Ok(None);
    };
    let spec: FleetSpec = decode(&row.try_get::<String>("", "spec_json")?)?;
    if spec.kind != FleetProviderKind::Gitlab {
        return Ok(None);
    }
    let section = spec
        .gitlab
        .ok_or_else(|| StoreError::Corrupt("GitLab target missing".into()))?;
    let target = section.target();
    let key = hex::encode(Sha256::digest(
        encode(&(
            "gitlab",
            fleet,
            &guard.incarnation,
            &target,
            &section.auth_profile_ref,
        ))?
        .as_bytes(),
    ));
    Ok(Some(ScopeSnapshot { key, target }))
}
