use super::{core_err, SqliteControlPlane};
use crate::entities::{
    auth::github_auth_profiles as auth,
    fleet::fleet_revision_pool_members,
    template::{profile_changes, template_profiles as template},
};
use sea_orm::{sea_query::Expr, ColumnTrait, EntityTrait, QueryFilter};
use shaula_core::error::CoreResult;
use shaula_core::registry::{ControlPlaneStore, MutationError, MutationFacts};

pub(super) async fn allows_references(
    tx: &sea_orm::DatabaseTransaction,
    facts: &MutationFacts,
    previous: Option<&crate::entities::fleet::fleet_revisions::Model>,
) -> crate::store::StoreResult<bool> {
    if let Some((key, rev, _, _)) = &facts.template {
        let unchanged = previous.is_some_and(|p| {
            p.template_profile_key.as_ref() == Some(key) && p.template_revision == Some(*rev)
        });
        if !unchanged
            && template::Entity::find_by_id(key.clone())
                .one(tx)
                .await?
                .is_some_and(|p| p.deletion_requested)
        {
            return Ok(false);
        }
    }
    if let Some((key, _)) = &facts.auth_desired {
        let unchanged = previous.is_some_and(|p| p.auth_desired_profile_key == *key);
        if !unchanged
            && auth::Entity::find_by_id(key.clone())
                .one(tx)
                .await?
                .is_some_and(|p| p.deletion_requested)
        {
            return Ok(false);
        }
    }
    for member in &facts.template_pool {
        let unchanged = if let Some(p) = previous {
            fleet_revision_pool_members::Entity::find()
                .filter(fleet_revision_pool_members::Column::FleetKey.eq(&facts.resource_key))
                .filter(fleet_revision_pool_members::Column::FleetRevision.eq(p.revision))
                .filter(
                    fleet_revision_pool_members::Column::TemplateProfileKey
                        .eq(&member.template_profile_key),
                )
                .filter(
                    fleet_revision_pool_members::Column::TemplateRevision
                        .eq(member.template_revision),
                )
                .one(tx)
                .await?
                .is_some()
        } else {
            false
        };
        if !unchanged
            && template::Entity::find_by_id(member.template_profile_key.clone())
                .one(tx)
                .await?
                .is_some_and(|p| p.deletion_requested)
        {
            return Ok(false);
        }
    }
    if let Some((key, revision)) = &facts.template_pool_ref {
        use sea_orm::{ConnectionTrait, DbBackend, Statement};
        // Pool DELETE and Fleet routing admission share the writer. A stale
        // admission snapshot cannot resurrect a tombstoned pool's authority.
        let live = tx
            .query_one(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "SELECT 1 FROM template_pools p JOIN template_pool_revisions r ON r.pool_key=p.key
             WHERE p.key=? AND r.revision=? AND p.desired_revision>=r.revision
             AND p.deletion_marker=0 AND p.tombstone=0",
                [key.clone().into(), (*revision).into()],
            ))
            .await?;
        if live.is_none() {
            return Ok(false);
        }
        let unchanged = previous.is_some_and(|p| {
            p.template_pool_ref.as_ref() == Some(key) && p.template_pool_revision == Some(*revision)
        });
        if !unchanged && tx.query_one(Statement::from_sql_and_values(DbBackend::Sqlite,
            "SELECT 1 FROM template_pool_members m LEFT JOIN template_profiles t ON t.key=m.template_profile_key
             WHERE m.pool_key=? AND m.pool_revision=? AND (t.key IS NULL OR t.deletion_requested=1) LIMIT 1",
            [key.clone().into(),(*revision).into()],
        )).await?.is_some() { return Ok(false); }
    }
    Ok(true)
}

impl SqliteControlPlane {
    pub(super) async fn commit_profile_retirement_impl(
        &self,
        facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>> {
        let tx = self.store.begin().await.map_err(core_err)?;
        let retired = if facts.resource_kind == "template_profile" {
            template::Entity::find_by_id(&facts.resource_key)
                .one(&tx)
                .await
                .map(|head| head.is_some_and(|h| h.status == "Retired"))
        } else {
            auth::Entity::find_by_id(&facts.resource_key)
                .one(&tx)
                .await
                .map(|head| head.is_some_and(|h| h.status == "Retired"))
        }
        .map_err(|e| core_err(e.into()))?;
        let status = if retired { "Retired" } else { "Retiring" };
        let changed = if facts.resource_kind == "template_profile" {
            template::Entity::update_many()
                .col_expr(template::Column::DeletionRequested, Expr::value(true))
                .col_expr(template::Column::Status, Expr::value(status))
                .col_expr(template::Column::UpdatedAt, Expr::value(facts.now))
                .filter(template::Column::Key.eq(&facts.resource_key))
                .filter(template::Column::Incarnation.eq(&facts.incarnation))
                .filter(template::Column::DesiredRevision.eq(facts.revision))
                .exec(&tx)
                .await
        } else {
            auth::Entity::update_many()
                .col_expr(auth::Column::DeletionRequested, Expr::value(true))
                .col_expr(auth::Column::Status, Expr::value(status))
                .col_expr(auth::Column::UpdatedAt, Expr::value(facts.now))
                .filter(auth::Column::Key.eq(&facts.resource_key))
                .filter(auth::Column::Incarnation.eq(&facts.incarnation))
                .filter(auth::Column::DesiredRevision.eq(facts.revision))
                .exec(&tx)
                .await
        }
        .map_err(|e| core_err(e.into()))?;
        if changed.rows_affected != 1 {
            tx.rollback().await.map_err(|e| core_err(e.into()))?;
            let current = if facts.resource_kind == "template_profile" {
                self.template_profile_get(&facts.resource_key).await?
            } else {
                self.auth_profile_get(&facts.resource_key).await?
            };
            return Ok(Err(current
                .map(|h| MutationError::PreconditionFailed {
                    current: (h.incarnation, h.desired_revision),
                })
                .unwrap_or(MutationError::NotFound)));
        }
        self.store
            .profile_change_insert(
                &tx,
                shaula_core::registry::ProfileChangeInsert {
                    id: facts.change.id.clone(),
                    resource_kind: facts.resource_kind.into(),
                    profile_key: facts.resource_key.clone(),
                    revision: Some(facts.revision),
                    kind: "Retire".into(),
                    now: facts.now,
                },
            )
            .await
            .map_err(core_err)?;
        profile_changes::Entity::update_many()
            .col_expr(
                profile_changes::Column::State,
                Expr::value(if retired { "Succeeded" } else { "Blocked" }),
            )
            .col_expr(
                profile_changes::Column::Reason,
                Expr::value((!retired).then_some("ResourceInUse")),
            )
            .filter(profile_changes::Column::Id.eq(&facts.change.id))
            .exec(&tx)
            .await
            .map_err(|e| core_err(e.into()))?;
        self.store
            .audit_append(
                &tx,
                shaula_core::registry::AuditAppend {
                    authentication: facts.authentication.clone(),
                    resource_kind: facts.resource_kind.into(),
                    action: "retire".into(),
                    actor: facts.actor.clone(),
                    resource_key: facts.resource_key.clone(),
                    revision: Some(facts.revision),
                    outcome: "accepted".into(),
                    detail_json: None,
                    now: facts.now,
                },
            )
            .await
            .map_err(core_err)?;
        self.store
            .outbox_enqueue(
                &tx,
                facts.resource_kind,
                &facts.outbox_topic,
                &facts.outbox_payload,
                facts.now,
            )
            .await
            .map_err(core_err)?;
        if let Some((key, hash, status, body)) = facts.idempotency {
            self.store
                .idempotency_store(
                    &tx,
                    shaula_core::registry::IdempotencyInsert {
                        operation: facts.idempotency_operation.into(),
                        principal: facts.actor.clone(),
                        id: format!("idem-{}", facts.change.id),
                        resource_kind: facts.resource_kind.into(),
                        resource_key: facts.resource_key,
                        idempotency_key: key,
                        request_hash: hash,
                        response_status: status,
                        response_body: Some(body),
                        now: facts.now,
                    },
                )
                .await
                .map_err(core_err)?;
        }
        tx.commit().await.map_err(|e| core_err(e.into()))?;
        Ok(Ok(()))
    }
}
