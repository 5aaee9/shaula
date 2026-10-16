//! Indexed token registry. Writer reservation encloses quota/replay and audit.
use crate::entities::access_tokens::{
    personal_access_tokens as tokens, personal_token_issues as issues,
};
use crate::Store;
use async_trait::async_trait;
use sea_orm::{
    sea_query::{Condition, Expr},
    ActiveValue::Set,
    ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use shaula_core::access_tokens::*;

fn unavailable<T>(_: T) -> TokenError {
    TokenError::Unavailable
}
fn record(row: tokens::Model) -> TokenResult<TokenRecord> {
    let mut record: TokenRecord = serde_json::from_str(&row.record_json).map_err(unavailable)?;
    record.last_used_at_ms = row.last_used_at;
    Ok(record)
}
async fn find(db: &impl ConnectionTrait, id: &str) -> TokenResult<Option<TokenRecord>> {
    tokens::Entity::find_by_id(id)
        .one(db)
        .await
        .map_err(unavailable)?
        .map(record)
        .transpose()
}
/// Tokens neither revoked nor expired at `now`.
fn active(now: i64) -> Condition {
    Condition::all()
        .add(tokens::Column::RevokedAt.is_null())
        .add(tokens::Column::ExpiresAt.gt(now))
}

#[async_trait]
impl TokenStore for Store {
    async fn get(&self, id: &str) -> TokenResult<Option<TokenRecord>> {
        find(self.connection(), id).await
    }

    async fn issue(
        &self,
        r: TokenRecord,
        key: &str,
        hash: &str,
        policy: &TokenPolicy,
        authentication: &shaula_core::registry::AuthenticationContext,
    ) -> TokenResult<(TokenRecord, bool)> {
        let tx = self.begin().await.map_err(unavailable)?;
        let replay = issues::Entity::find_by_id((r.owner_principal.clone(), key.to_owned()))
            .one(&tx)
            .await
            .map_err(unavailable)?;
        if let Some(issued) = replay {
            if issued.request_hash != hash {
                return Err(TokenError::Conflict);
            }
            let existing = find(&tx, &issued.token_id)
                .await?
                .ok_or(TokenError::Unavailable)?;
            tx.commit().await.map_err(unavailable)?;
            return Ok((existing, false));
        }
        let now = r.created_at_ms;
        if r.expires_at_ms.saturating_sub(now) > (policy.max_ttl_secs * 1000) as i64 {
            return Err(TokenError::Invalid);
        }
        let since = now.saturating_sub(60_000);
        let recent = issues::Entity::find().filter(issues::Column::CreatedAt.gt(since));
        let global = recent.clone().count(&tx).await.map_err(unavailable)?;
        let personal = recent
            .filter(issues::Column::Owner.eq(r.owner_principal.as_str()))
            .count(&tx)
            .await
            .map_err(unavailable)?;
        if global >= 100 || personal >= 5 {
            return Err(TokenError::RateLimited);
        }
        let live = tokens::Entity::find().filter(active(now));
        let total = live.clone().count(&tx).await.map_err(unavailable)?;
        let owned = live
            .filter(tokens::Column::Owner.eq(r.owner_principal.as_str()))
            .count(&tx)
            .await
            .map_err(unavailable)?;
        if total >= policy.max_active_total || owned >= policy.max_active_per_principal {
            return Err(TokenError::Quota);
        }
        let json = serde_json::to_string(&r).map_err(unavailable)?;
        tokens::Entity::insert(tokens::ActiveModel {
            id: Set(r.id.clone()),
            owner: Set(r.owner_principal.clone()),
            created_at: Set(now),
            expires_at: Set(r.expires_at_ms),
            record_json: Set(json),
            ..Default::default()
        })
        .exec_without_returning(&tx)
        .await
        .map_err(unavailable)?;
        issues::Entity::insert(issues::ActiveModel {
            owner: Set(r.owner_principal.clone()),
            idempotency_key: Set(key.to_owned()),
            request_hash: Set(hash.to_owned()),
            token_id: Set(r.id.clone()),
            created_at: Set(now),
        })
        .exec_without_returning(&tx)
        .await
        .map_err(unavailable)?;
        audit(self, &tx, &r, "issue", now, authentication).await?;
        tx.commit().await.map_err(unavailable)?;
        Ok((r, true))
    }

    async fn list(
        &self,
        owner: &str,
        before: Option<(i64, String)>,
        limit: usize,
    ) -> TokenResult<Vec<TokenRecord>> {
        let mut query = tokens::Entity::find().filter(tokens::Column::Owner.eq(owner));
        if let Some((at, id)) = before {
            query = query.filter(
                Condition::any().add(tokens::Column::CreatedAt.lt(at)).add(
                    Condition::all()
                        .add(tokens::Column::CreatedAt.eq(at))
                        .add(tokens::Column::Id.lt(id)),
                ),
            );
        }
        query
            .order_by_desc(tokens::Column::CreatedAt)
            .order_by_desc(tokens::Column::Id)
            .limit(limit.min(100) as u64)
            .all(self.connection())
            .await
            .map_err(unavailable)?
            .into_iter()
            .map(record)
            .collect()
    }

    async fn revoke(
        &self,
        actor: &shaula_core::registry::Actor,
        id: &str,
        revision: Option<i64>,
        now: i64,
    ) -> TokenResult<()> {
        let tx = self.begin().await.map_err(unavailable)?;
        let owner = actor.name.as_str();
        let mut r = find(&tx, id)
            .await?
            .filter(|r| r.owner_principal == owner)
            .ok_or(TokenError::NotFound)?;
        if revision.is_some_and(|v| v != r.revision) {
            return Err(TokenError::PreconditionFailed);
        }
        if r.revoked_at_ms.is_none() {
            r.revoked_at_ms = Some(now);
            r.revoked_by_principal = Some(owner.to_owned());
            r.revision += 1;
            let json = serde_json::to_string(&r).map_err(unavailable)?;
            tokens::Entity::update_many()
                .col_expr(tokens::Column::RevokedAt, Expr::value(now))
                .col_expr(tokens::Column::RecordJson, Expr::value(json))
                .filter(tokens::Column::Id.eq(id))
                .exec(&tx)
                .await
                .map_err(unavailable)?;
            audit(self, &tx, &r, "revoke", now, &actor.authentication).await?;
        }
        tx.commit().await.map_err(unavailable)?;
        Ok(())
    }

    async fn touch(&self, id: &str, now: i64) -> TokenResult<()> {
        tokens::Entity::update_many()
            .col_expr(tokens::Column::LastUsedAt, Expr::value(now))
            .filter(tokens::Column::Id.eq(id))
            .filter(
                Condition::any()
                    .add(tokens::Column::LastUsedAt.is_null())
                    .add(tokens::Column::LastUsedAt.lte(now.saturating_sub(60_000))),
            )
            .exec(self.connection())
            .await
            .map_err(unavailable)?;
        Ok(())
    }
}

async fn audit(
    store: &Store,
    tx: &sea_orm::DatabaseTransaction,
    r: &TokenRecord,
    action: &str,
    now: i64,
    authentication: &shaula_core::registry::AuthenticationContext,
) -> TokenResult<()> {
    store
        .audit_append(
            tx,
            shaula_core::registry::AuditAppend {
                authentication: authentication.clone(),
                resource_kind: "personal_access_token".into(),
                action: action.into(),
                actor: r.owner_principal.clone(),
                resource_key: r.id.clone(),
                revision: Some(r.revision),
                outcome: "committed".into(),
                detail_json: None,
                now,
            },
        )
        .await
        .map_err(unavailable)
}
