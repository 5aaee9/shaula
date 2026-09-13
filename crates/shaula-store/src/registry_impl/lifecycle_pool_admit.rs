//! Shared-pool generation admission helpers (spec 0037 §4–§6): member
//! loading, pool-wide cap counting and the eligible draw set.

use sea_orm::{ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, QueryOrder};

use crate::entities::fleet::fleet_revisions;
use crate::entities::lifecycle::runner_generations;
use crate::entities::template_pool::{template_pool_members, template_pool_revisions};

use super::core_err;
use shaula_core::error::CoreResult;
use shaula_core::template_pool::{FleetPoolRef, PoolFailurePolicy};

/// The normalized member shape both routing sources map into.
pub(crate) struct AdmitMember {
    pub member_key: String,
    pub template_profile_key: String,
    pub template_revision: i64,
    pub template_artifact_digest: String,
    pub template_attestation_id: String,
    pub template_inputs_json: String,
    pub inputs_digest: String,
    pub weight: i64,
    pub max_runners: Option<i64>,
}

pub(crate) struct PoolRouting {
    pub members: Vec<AdmitMember>,
    pub failure_policy: PoolFailurePolicy,
    /// The shared-pool scope when this fleet revision routes through a
    /// pool revision; `None` for inline pools.
    pub pool_scope: Option<FleetPoolRef>,
}

/// Loads the routing context for one fleet revision: shared-pool member
/// rows when the revision froze a pool reference, else inline rows.
pub(crate) async fn load_routing(
    tx: &DatabaseTransaction,
    fleet_key: &str,
    fleet_revision: i64,
    inline_members: Vec<AdmitMember>,
    inline_policy: PoolFailurePolicy,
) -> CoreResult<PoolRouting> {
    let Some(row) = fleet_revisions::Entity::find()
        .filter(fleet_revisions::Column::FleetKey.eq(fleet_key))
        .filter(fleet_revisions::Column::Revision.eq(fleet_revision))
        .one(tx)
        .await
        .map_err(|e| core_err(e.into()))?
    else {
        return Ok(PoolRouting {
            members: inline_members,
            failure_policy: inline_policy,
            pool_scope: None,
        });
    };
    let Some((pool_key, pool_revision)) = row
        .template_pool_ref
        .clone()
        .zip(row.template_pool_revision)
    else {
        return Ok(PoolRouting {
            members: inline_members,
            failure_policy: inline_policy,
            pool_scope: None,
        });
    };
    let members = template_pool_members::Entity::find()
        .filter(template_pool_members::Column::PoolKey.eq(&pool_key))
        .filter(template_pool_members::Column::PoolRevision.eq(pool_revision))
        .all(tx)
        .await
        .map_err(|e| core_err(e.into()))?
        .into_iter()
        .map(|m| AdmitMember {
            member_key: m.member_key,
            template_profile_key: m.template_profile_key,
            template_revision: m.template_revision,
            template_artifact_digest: m.template_artifact_digest,
            template_attestation_id: m.template_attestation_id,
            template_inputs_json: m.template_inputs_json,
            inputs_digest: m.inputs_digest,
            weight: m.weight,
            max_runners: m.max_runners,
        })
        .collect();
    let failure_policy = template_pool_revisions::Entity::find()
        .filter(template_pool_revisions::Column::PoolKey.eq(&pool_key))
        .filter(template_pool_revisions::Column::Revision.eq(pool_revision))
        .one(tx)
        .await
        .map_err(|e| core_err(e.into()))?
        .map(|row| {
            if row.failure_policy == "redistribute" {
                PoolFailurePolicy::Redistribute
            } else {
                PoolFailurePolicy::Backpressure
            }
        })
        .unwrap_or_default();
    Ok(PoolRouting {
        members,
        failure_policy,
        pool_scope: Some((pool_key, pool_revision)),
    })
}

/// Live (non-Destroyed) generations of every fleet whose fleet revision
/// froze this pool revision — the pool-wide occupancy the member caps
/// count against (spec 0037 §6).
pub(crate) async fn pool_wide_generations(
    tx: &DatabaseTransaction,
    pool_scope: &FleetPoolRef,
) -> CoreResult<Vec<runner_generations::Model>> {
    let referencing = fleet_revisions::Entity::find()
        .filter(fleet_revisions::Column::TemplatePoolRef.eq(pool_scope.0.clone()))
        .filter(fleet_revisions::Column::TemplatePoolRevision.eq(pool_scope.1))
        .all(tx)
        .await
        .map_err(|e| core_err(e.into()))?;
    let mut generations = Vec::new();
    for revision in referencing {
        generations.extend(
            runner_generations::Entity::find()
                .filter(runner_generations::Column::FleetKey.eq(&revision.fleet_key))
                .filter(runner_generations::Column::FleetRevision.eq(revision.revision))
                .filter(runner_generations::Column::State.ne("Destroyed"))
                .all(tx)
                .await
                .map_err(|e| core_err(e.into()))?,
        );
    }
    Ok(generations)
}

/// The inputs a replayed generation freezes: pool member rows when its
/// fleet revision routed through a shared pool.
pub(crate) async fn replay_member_inputs(
    tx: &DatabaseTransaction,
    fleet_key: &str,
    fleet_revision: i64,
    member_key: &str,
    inline_inputs: Option<serde_json::Map<String, serde_json::Value>>,
) -> CoreResult<serde_json::Map<String, serde_json::Value>> {
    let row = fleet_revisions::Entity::find()
        .filter(fleet_revisions::Column::FleetKey.eq(fleet_key))
        .filter(fleet_revisions::Column::Revision.eq(fleet_revision))
        .order_by_desc(fleet_revisions::Column::Revision)
        .one(tx)
        .await
        .map_err(|e| core_err(e.into()))?;
    if let Some((pool_key, pool_revision)) =
        row.and_then(|r| r.template_pool_ref.clone().zip(r.template_pool_revision))
    {
        if let Some(member) = template_pool_members::Entity::find()
            .filter(template_pool_members::Column::PoolKey.eq(&pool_key))
            .filter(template_pool_members::Column::PoolRevision.eq(pool_revision))
            .filter(template_pool_members::Column::MemberKey.eq(member_key))
            .one(tx)
            .await
            .map_err(|e| core_err(e.into()))?
        {
            return Ok(serde_json::from_str(&member.template_inputs_json).unwrap_or_default());
        }
        return Ok(serde_json::Map::new());
    }
    Ok(inline_inputs.unwrap_or_default())
}
