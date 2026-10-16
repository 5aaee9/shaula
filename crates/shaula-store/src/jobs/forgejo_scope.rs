//! One captured authority check for snapshots and optional task-history reads.
use super::{decode, encode, guarded_spec};
use crate::{StoreError, StoreResult};
use sea_orm::DatabaseTransaction;
use sha2::{Digest, Sha256};
use shaula_core::{
    fleet::{FleetProviderKind, FleetSpec},
    forgejo::ForgejoTarget,
    registry::FleetRuntimeGuard,
};

pub(super) struct ScopeSnapshot {
    pub key: String,
    pub target: ForgejoTarget,
}

pub(super) async fn current(
    tx: &DatabaseTransaction,
    fleet: &str,
    guard: &FleetRuntimeGuard,
) -> StoreResult<Option<ScopeSnapshot>> {
    let Some(json) = guarded_spec(tx, fleet, guard).await? else {
        return Ok(None);
    };
    let spec: FleetSpec = decode(&json)?;
    if spec.kind != FleetProviderKind::Forgejo {
        return Ok(None);
    }
    let section = spec
        .forgejo
        .ok_or_else(|| StoreError::Corrupt("Forgejo target missing".into()))?;
    let target = section.target();
    let key = hex::encode(Sha256::digest(
        encode(&(
            fleet,
            &guard.incarnation,
            &target,
            &section.auth_profile_ref,
        ))?
        .as_bytes(),
    ));
    Ok(Some(ScopeSnapshot { key, target }))
}
