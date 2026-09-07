use super::{core_err, SqliteControlPlane};
use crate::entities::{
    auth::github_auth_profiles as auth,
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
    Ok(true)
}

impl SqliteControlPlane {
    pub(super) async fn commit_profile_retirement_impl(
        &self,
        facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>> {
        let tx = self.store.begin().await.map_err(core_err)?;
        let changed = if facts.resource_kind == "template_profile" {
            template::Entity::update_many()
                .col_expr(template::Column::DeletionRequested, Expr::value(true))
                .col_expr(template::Column::Status, Expr::value("Retiring"))
                .col_expr(template::Column::UpdatedAt, Expr::value(facts.now))
                .filter(template::Column::Key.eq(&facts.resource_key))
                .filter(template::Column::Incarnation.eq(&facts.incarnation))
                .filter(template::Column::DesiredRevision.eq(facts.revision))
                .exec(&tx)
                .await
        } else {
            auth::Entity::update_many()
                .col_expr(auth::Column::DeletionRequested, Expr::value(true))
                .col_expr(auth::Column::Status, Expr::value("Retiring"))
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
            .col_expr(profile_changes::Column::State, Expr::value("Blocked"))
            .col_expr(
                profile_changes::Column::Reason,
                Expr::value("ResourceInUse"),
            )
            .filter(profile_changes::Column::Id.eq(&facts.change.id))
            .exec(&tx)
            .await
            .map_err(|e| core_err(e.into()))?;
        self.store
            .audit_append(
                &tx,
                shaula_core::registry::AuditAppend {
                    resource_kind: facts.resource_kind.into(),
                    action: "retire".into(),
                    actor: facts.actor,
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
