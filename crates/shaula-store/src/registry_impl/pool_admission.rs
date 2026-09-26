//! `LifecycleStore` port implementation plus the Store helpers shared by
//! the idempotency and handoff flows.

use shaula_core::diagnostics::*;

use shaula_core::error::CoreResult;

use super::core_err;
use super::lifecycle_support::map_generation;
use super::SqliteControlPlane;
use crate::entities::{
    fleet::{fleet_revision_pool_members, fleets},
    lifecycle::runner_generations,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

impl SqliteControlPlane {
    pub(super) async fn generation_admit_pool_impl(
        &self,
        mut record: shaula_core::registry::GenerationRecord,
        guard: &shaula_core::registry::FleetRuntimeGuard,
    ) -> CoreResult<Option<shaula_core::template_pool::PoolGenerationAdmission>> {
        let mut diagnostic = Capture::start(
            Some(self.store.diagnostics.clone()),
            Guard::fleet(&record.fleet_key, guard),
            Lane::Admission,
            QuestionId::ScaleUp,
            record.created_at,
        );
        let tx = self.store.begin().await.map_err(core_err)?;
        let Some(fleet) = fleets::Entity::find_by_id(record.fleet_key.clone())
            .one(&tx)
            .await
            .map_err(|e| core_err(e.into()))?
        else {
            tx.rollback().await.ok();
            return Ok(None);
        };
        if fleet.incarnation != guard.incarnation
            || fleet.desired_revision != guard.desired_revision
            || fleet.mutation_fence != guard.mutation_fence
            || fleet.deletion_marker
            || fleet.tombstone
        {
            tx.rollback().await.ok();
            return Ok(None);
        }
        if let Some(existing) = runner_generations::Entity::find_by_id(record.id.clone())
            .one(&tx)
            .await
            .map_err(|e| core_err(e.into()))?
        {
            record = map_generation(existing);
            let inputs = if let Some(key) = record.pool_member_key.as_deref() {
                let inline = match fleet_revision_pool_members::Entity::find()
                    .filter(fleet_revision_pool_members::Column::FleetKey.eq(&record.fleet_key))
                    .filter(
                        fleet_revision_pool_members::Column::FleetRevision
                            .eq(record.fleet_revision),
                    )
                    .filter(fleet_revision_pool_members::Column::MemberKey.eq(key))
                    .one(&tx)
                    .await
                    .map_err(|e| core_err(e.into()))?
                {
                    Some(member) => Some(
                        super::mapping::template_inputs_from_json(&member.template_inputs_json)
                            .map_err(core_err)?,
                    ),
                    None => None,
                };
                super::lifecycle_pool_admit::replay_member_inputs(
                    &tx,
                    &record.fleet_key,
                    record.fleet_revision,
                    key,
                    inline,
                )
                .await?
            } else {
                serde_json::Map::new()
            };
            tx.rollback().await.map_err(|e| core_err(e.into()))?;
            return Ok(Some(shaula_core::template_pool::PoolGenerationAdmission {
                generation: record,
                template_inputs: inputs,
            }));
        }
        let revision_row = crate::entities::fleet::fleet_revisions::Entity::find()
            .filter(crate::entities::fleet::fleet_revisions::Column::FleetKey.eq(&record.fleet_key))
            .filter(
                crate::entities::fleet::fleet_revisions::Column::Revision.eq(record.fleet_revision),
            )
            .one(&tx)
            .await
            .map_err(|e| core_err(e.into()))?
            .ok_or_else(|| {
                core_err(crate::store::StoreError::Corrupt(format!(
                    "fleet revision {}:{} is missing",
                    record.fleet_key, record.fleet_revision
                )))
            })?;
        let spec = serde_json::from_str::<shaula_core::fleet::FleetSpec>(&revision_row.spec_json)
            .map_err(|error| {
            core_err(crate::store::StoreError::Corrupt(format!(
                "fleet revision {}:{} has invalid spec: {error}",
                record.fleet_key, record.fleet_revision
            )))
        })?;
        let inline_policy = spec
            .template_pool
            .map(|pool| pool.failure_policy)
            .unwrap_or_default();
        let inline_members = fleet_revision_pool_members::Entity::find()
            .filter(fleet_revision_pool_members::Column::FleetKey.eq(&record.fleet_key))
            .filter(fleet_revision_pool_members::Column::FleetRevision.eq(record.fleet_revision))
            .all(&tx)
            .await
            .map_err(|e| core_err(e.into()))?;
        // Routing source: shared-pool member rows when the fleet revision
        // froze a pool reference, else the inline rows (spec 0037 §4).
        let routing = super::lifecycle_pool_admit::load_routing(
            &tx,
            &record.fleet_key,
            record.fleet_revision,
            inline_members
                .into_iter()
                .map(|m| {
                    Ok(super::lifecycle_pool_admit::AdmitMember {
                        member_key: m.member_key,
                        template_profile_key: m.template_profile_key,
                        template_revision: m.template_revision,
                        template_artifact_digest: m.template_artifact_digest,
                        template_attestation_id: m.template_attestation_id,
                        template_inputs: super::mapping::template_inputs_from_json(
                            &m.template_inputs_json,
                        )
                        .map_err(core_err)?,
                        inputs_digest: m.inputs_digest,
                        weight: super::mapping::pool_member_weight(m.weight).map_err(core_err)?,
                        max_runners: m.max_runners,
                    })
                })
                .collect::<CoreResult<Vec<_>>>()?,
            inline_policy,
        )
        .await?;
        if let Some(ticket) = &mut diagnostic.0 {
            ticket.observation.question.pool = Some(DiagnosticPool {
                mode: if routing.pool_scope.is_some() {
                    RoutingMode::Shared
                } else {
                    RoutingMode::Inline
                },
                scope: if routing.pool_scope.is_some() {
                    OccupancyScope::PoolRevision
                } else {
                    OccupancyScope::Fleet
                },
                pool_revision: routing.pool_scope.as_ref().map(|s| s.1.to_string()),
                members: vec![],
                selected_member: None,
                selection: SelectionOutcome::NotEvaluated,
                truncated: false,
            });
        }
        let members = routing.members;
        if members.is_empty() {
            diagnostic.reason(Code::PoolNoEligibleMember, StageId::Pool, true);
            if let Some(pool) = diagnostic
                .0
                .as_mut()
                .and_then(|d| d.observation.question.pool.as_mut())
            {
                pool.selection = SelectionOutcome::NoEligibleMember;
            }
            tx.rollback().await.map_err(|e| core_err(e.into()))?;
            return Ok(None);
        }
        let occupancy = runner_generations::Entity::find()
            .filter(runner_generations::Column::FleetKey.eq(&record.fleet_key))
            .all(&tx)
            .await
            .map_err(|e| core_err(e.into()))?
            .iter()
            .filter(|g| g.state != "Destroyed")
            .count() as i64;
        if occupancy >= spec.capacity.max_runners {
            diagnostic.reason(Code::CapacityOccupancyLimit, StageId::Capacity, true);
            tx.rollback().await.map_err(|e| core_err(e.into()))?;
            return Ok(None);
        }
        // Pool-wide occupancy when the fleet routes through a shared pool:
        // member caps count every fleet referencing that pool revision
        // (spec 0037 §6); inline pools keep their fleet-local accounting.
        let generations = if let Some(scope) = &routing.pool_scope {
            super::lifecycle_pool_admit::pool_wide_generations(&tx, scope).await?
        } else {
            runner_generations::Entity::find()
                .filter(runner_generations::Column::FleetKey.eq(&record.fleet_key))
                .all(&tx)
                .await
                .map_err(|e| core_err(e.into()))?
                .into_iter()
                .filter(|g| g.state != "Destroyed")
                .collect()
        };
        let mut eligible = Vec::new();
        let mut member_at_capacity = false;
        for member in members {
            let used = generations
                .iter()
                .filter(|g| g.pool_member_key.as_deref() == Some(member.member_key.as_str()))
                .count() as i64;
            if let Some(pool) = diagnostic
                .0
                .as_mut()
                .and_then(|d| d.observation.question.pool.as_mut())
            {
                if pool.members.len() < 128 {
                    pool.members.push(DiagnosticPoolMember {
                        key: member.member_key.clone(),
                        weight: member.weight.to_string(),
                        cap: member.max_runners.map(|v| v.to_string()),
                        occupancy: used.to_string(),
                        excluded_at_cap: member.max_runners.is_some_and(|cap| used >= cap),
                    });
                } else {
                    pool.truncated = true;
                }
            }
            if member.max_runners.is_none_or(|cap| used < cap) {
                eligible.push((member, used));
            } else {
                member_at_capacity = true;
            }
        }
        if member_at_capacity {
            diagnostic.reason(Code::PoolMemberAtCap, StageId::Pool, false);
        }
        if routing.pool_scope.is_some() {
            // Shared pools (spec 0037 §6): a capped member leaves the
            // eligible draw set under BOTH policies; weights renormalize
            // over the survivors, and only an empty eligible set
            // backpressures.
        } else if member_at_capacity
            && routing.failure_policy == shaula_core::template_pool::PoolFailurePolicy::Backpressure
        {
            diagnostic.reason(Code::PoolInlineBackpressure, StageId::Pool, true);
            if let Some(pool) = diagnostic
                .0
                .as_mut()
                .and_then(|d| d.observation.question.pool.as_mut())
            {
                pool.selection = SelectionOutcome::Backpressure;
            }
            // Inline pools keep the spec 0029 contract: backpressure keeps
            // the configured pool intact when any route is at capacity.
            tx.rollback().await.map_err(|e| core_err(e.into()))?;
            return Ok(None);
        }
        if eligible.is_empty() {
            diagnostic.reason(Code::PoolNoEligibleMember, StageId::Pool, true);
            if let Some(pool) = diagnostic
                .0
                .as_mut()
                .and_then(|d| d.observation.question.pool.as_mut())
            {
                pool.selection = SelectionOutcome::NoEligibleMember;
            }
            tx.rollback().await.map_err(|e| core_err(e.into()))?;
            return Ok(None);
        }
        diagnostic.pass(StageId::Pool);
        let weights: Vec<u32> = eligible.iter().map(|(member, _)| member.weight).collect();
        let index = loop {
            if let Some(index) = shaula_core::template_pool::weighted_member_index(
                &weights,
                uuid::Uuid::new_v4().as_u128(),
            ) {
                break index;
            }
        };
        let selected = eligible
            .into_iter()
            .nth(index)
            .ok_or_else(|| {
                core_err(crate::store::StoreError::Corrupt(
                    "weighted pool selection index missing".into(),
                ))
            })?
            .0;
        record.pool_member_key = Some(selected.member_key.clone());
        record.template_profile_key = selected.template_profile_key.clone();
        record.template_revision = selected.template_revision;
        record.template_artifact_digest = selected.template_artifact_digest.clone();
        record.attestation_id = selected.template_attestation_id.clone();
        record.inputs_digest = selected.inputs_digest.clone();
        crate::Store::generation_insert_on(&tx, record.clone())
            .await
            .map_err(core_err)?;
        let template_inputs = selected.template_inputs;
        tx.commit().await.map_err(|e| core_err(e.into()))?;
        if let Some(pool) = diagnostic
            .0
            .as_mut()
            .and_then(|d| d.observation.question.pool.as_mut())
        {
            pool.selected_member = Some(selected.member_key);
            pool.selection = SelectionOutcome::Admitted;
        }
        diagnostic.pass(StageId::ExecutionAdmission);
        diagnostic.outcome(Outcome::Progressing);
        Ok(Some(shaula_core::template_pool::PoolGenerationAdmission {
            generation: record,
            template_inputs,
        }))
    }
    pub(super) async fn generation_insert_guarded_impl(
        &self,
        record: shaula_core::registry::GenerationRecord,
        guard: &shaula_core::registry::FleetRuntimeGuard,
    ) -> CoreResult<bool> {
        let mut diagnostic = Capture::start(
            Some(self.store.diagnostics.clone()),
            Guard::fleet(&record.fleet_key, guard),
            Lane::Admission,
            QuestionId::ScaleUp,
            record.created_at,
        );
        if let Some(d) = &mut diagnostic.0 {
            d.observation.question.pool = Some(DiagnosticPool {
                mode: RoutingMode::Single,
                scope: OccupancyScope::Fleet,
                pool_revision: None,
                members: vec![],
                selected_member: None,
                selection: SelectionOutcome::NotEvaluated,
                truncated: false,
            });
        }
        let tx = self.store.begin().await.map_err(core_err)?;
        let Some(fleet) = fleets::Entity::find_by_id(record.fleet_key.clone())
            .one(&tx)
            .await
            .map_err(|e| core_err(e.into()))?
        else {
            tx.rollback().await.ok();
            return Ok(false);
        };
        if fleet.incarnation != guard.incarnation
            || fleet.desired_revision != guard.desired_revision
            || fleet.mutation_fence != guard.mutation_fence
            || fleet.deletion_marker
            || fleet.tombstone
        {
            tx.rollback().await.ok();
            return Ok(false);
        }
        crate::Store::generation_insert_on(&tx, record)
            .await
            .map_err(core_err)?;
        tx.commit().await.map_err(|e| core_err(e.into()))?;
        if let Some(pool) = diagnostic
            .0
            .as_mut()
            .and_then(|d| d.observation.question.pool.as_mut())
        {
            pool.selection = SelectionOutcome::Admitted;
        }
        diagnostic.pass(StageId::ExecutionAdmission);
        diagnostic.outcome(Outcome::Progressing);
        Ok(true)
    }
}
