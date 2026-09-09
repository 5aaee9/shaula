//! Exact Active credential inheritance is serialized with publication and replay.

use sea_orm::{ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter};
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::registry::{AuthRevisionRow, MutationAccepted, MutationError, MutationFacts};

use crate::entities::auth::github_auth_profiles;
use crate::entities::shared::idempotency_records;

use super::{core_err, SqliteControlPlane};

impl SqliteControlPlane {
    pub(crate) async fn commit_auth_policy_update_impl(
        &self,
        facts: MutationFacts,
        base_revision: i64,
        policy_json: String,
    ) -> CoreResult<Result<MutationAccepted, MutationError>> {
        // begin() reserves SQLite's writer before the first authority read.
        let tx = self.store.begin().await.map_err(core_err)?;
        match policy_replay(&tx, &facts).await? {
            Ok(Some(accepted)) => return Ok(Ok(accepted)),
            Err(error) => return Ok(Err(error)),
            Ok(None) => {}
        }
        let Some(head) = github_auth_profiles::Entity::find_by_id(facts.resource_key.clone())
            .one(&tx)
            .await
            .map_err(|error| core_err(error.into()))?
        else {
            return Ok(Err(MutationError::NotFound));
        };
        if head.incarnation != facts.incarnation
            || head.desired_revision.checked_add(1) != Some(facts.revision)
        {
            return Ok(Err(MutationError::PreconditionFailed {
                current: (head.incarnation, head.desired_revision),
            }));
        }
        if head.deletion_requested || matches!(head.status.as_str(), "Retiring" | "Retired") {
            return Ok(Err(MutationError::RetirementBlocked {
                reason: "profile is retiring".into(),
            }));
        }
        if !matches!(head.status.as_str(), "Active" | "Validating")
            || base_revision <= 0
            || head.active_revision != Some(base_revision)
        {
            return Ok(Err(MutationError::IdentityConflict));
        }
        let Some(base) = self
            .store
            .auth_revision_get_tx(&tx, &facts.resource_key, base_revision)
            .await
            .map_err(core_err)?
        else {
            return Ok(Err(MutationError::IdentityConflict));
        };
        if base.schema_version != 2
            || base.kind != "github_app"
            || base.state != "Active"
            || base.app_id.as_ref().is_none_or(|id| id.is_empty())
            || base.credential_bytes.is_empty()
        {
            return Ok(Err(MutationError::IdentityConflict));
        }
        let accepted = MutationAccepted {
            etag: format!("{}:{}", facts.incarnation, facts.revision),
            change: facts.change.clone(),
            no_op: false,
        };
        let credential = AuthRevisionRow {
            profile_key: facts.resource_key.clone(),
            revision: facts.revision,
            state: "Validating".into(),
            reason: None,
            kind: base.kind,
            app_id: base.app_id,
            schema_version: base.schema_version,
            policy_json: Some(policy_json),
            validation_snapshot_json: None,
        };
        // This immutable row's complete credential is copied while the same
        // transaction still excludes activation, retirement and cleanup races.
        match self
            .commit_auth_candidate(
                tx,
                facts,
                credential,
                &base.credential_bytes,
                Some(base_revision),
            )
            .await?
        {
            Ok(()) => Ok(Ok(accepted)),
            Err(error) => Ok(Err(error)),
        }
    }
}

async fn policy_replay(
    tx: &DatabaseTransaction,
    facts: &MutationFacts,
) -> CoreResult<Result<Option<MutationAccepted>, MutationError>> {
    let Some((key, hash, _, _)) = &facts.idempotency else {
        return Ok(Ok(None));
    };
    let Some(record) = idempotency_records::Entity::find()
        .filter(idempotency_records::Column::ResourceKind.eq(facts.resource_kind))
        .filter(idempotency_records::Column::ResourceKey.eq(&facts.resource_key))
        .filter(idempotency_records::Column::IdempotencyKey.eq(key))
        .one(tx)
        .await
        .map_err(|error| core_err(error.into()))?
    else {
        return Ok(Ok(None));
    };
    if record.request_hash != *hash {
        return Ok(Err(MutationError::IdempotencyConflict));
    }
    let accepted = record
        .response_body
        .as_deref()
        .and_then(|body| serde_json::from_str(body).ok())
        .ok_or_else(|| {
            CoreError::new(
                ReasonCode::Internal,
                "stored policy publication result is invalid",
            )
        })?;
    Ok(Ok(Some(accepted)))
}
