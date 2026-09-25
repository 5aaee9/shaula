//! Indexed token registry. Writer reservation encloses quota/replay and audit.
use crate::Store;
use async_trait::async_trait;
use sea_orm::{ConnectionTrait, DatabaseBackend, QueryResult, Statement, Value};
use shaula_core::access_tokens::*;

fn sql(text: &str, values: Vec<Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Sqlite, text, values)
}
fn unavailable<T>(_: T) -> TokenError {
    TokenError::Unavailable
}
fn record(row: QueryResult) -> TokenResult<TokenRecord> {
    let json: String = row.try_get("", "record_json").map_err(unavailable)?;
    let mut record: TokenRecord = serde_json::from_str(&json).map_err(unavailable)?;
    record.last_used_at_ms = row.try_get("", "last_used_at").map_err(unavailable)?;
    Ok(record)
}
async fn find(db: &impl ConnectionTrait, id: &str) -> TokenResult<Option<TokenRecord>> {
    db.query_one(sql(
        "SELECT record_json,last_used_at FROM personal_access_tokens WHERE id=?",
        vec![id.into()],
    ))
    .await
    .map_err(unavailable)?
    .map(record)
    .transpose()
}
async fn count(db: &impl ConnectionTrait, text: &str, values: Vec<Value>) -> TokenResult<i64> {
    db.query_one(sql(text, values))
        .await
        .map_err(unavailable)?
        .ok_or(TokenError::Unavailable)?
        .try_get("", "n")
        .map_err(unavailable)
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
        if let Some(row)=tx.query_one(sql("SELECT request_hash,token_id FROM personal_token_issues WHERE owner=? AND idempotency_key=?",vec![r.owner_principal.clone().into(),key.into()])).await.map_err(unavailable)? {
            let stored:String=row.try_get("","request_hash").map_err(unavailable)?;
            if stored!=hash { return Err(TokenError::Conflict); }
            let id:String=row.try_get("","token_id").map_err(unavailable)?;
            let existing=find(&tx,&id).await?.ok_or(TokenError::Unavailable)?;
            tx.commit().await.map_err(unavailable)?;
            return Ok((existing,false));
        }
        let now = r.created_at_ms;
        if r.expires_at_ms.saturating_sub(now) > (policy.max_ttl_secs * 1000) as i64 {
            return Err(TokenError::Invalid);
        }
        let since = now.saturating_sub(60_000);
        let global = count(
            &tx,
            "SELECT COUNT(*) AS n FROM personal_token_issues WHERE created_at>?",
            vec![since.into()],
        )
        .await?;
        let personal = count(
            &tx,
            "SELECT COUNT(*) AS n FROM personal_token_issues WHERE created_at>? AND owner=?",
            vec![since.into(), r.owner_principal.clone().into()],
        )
        .await?;
        if global >= 100 || personal >= 5 {
            return Err(TokenError::RateLimited);
        }
        let total=count(&tx,"SELECT COUNT(*) AS n FROM personal_access_tokens WHERE revoked_at IS NULL AND expires_at>?",vec![now.into()]).await?;
        let owned=count(&tx,"SELECT COUNT(*) AS n FROM personal_access_tokens WHERE revoked_at IS NULL AND expires_at>? AND owner=?",vec![now.into(),r.owner_principal.clone().into()]).await?;
        if total as u64 >= policy.max_active_total
            || owned as u64 >= policy.max_active_per_principal
        {
            return Err(TokenError::Quota);
        }
        let json = serde_json::to_string(&r).map_err(unavailable)?;
        tx.execute(sql("INSERT INTO personal_access_tokens(id,owner,created_at,expires_at,record_json) VALUES(?,?,?,?,?)",
            vec![r.id.clone().into(),r.owner_principal.clone().into(),now.into(),r.expires_at_ms.into(),json.into()])).await.map_err(unavailable)?;
        tx.execute(sql("INSERT INTO personal_token_issues(owner,idempotency_key,request_hash,token_id,created_at) VALUES(?,?,?,?,?)",
            vec![r.owner_principal.clone().into(),key.into(),hash.into(),r.id.clone().into(),now.into()])).await.map_err(unavailable)?;
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
        let mut values = vec![owner.into()];
        let mut query =
            "SELECT record_json,last_used_at FROM personal_access_tokens WHERE owner=?".to_owned();
        if let Some((at, id)) = before {
            query.push_str(" AND (created_at < ? OR (created_at = ? AND id < ?))");
            values.extend([at.into(), at.into(), id.into()]);
        }
        query.push_str(" ORDER BY created_at DESC,id DESC LIMIT ?");
        values.push((limit.min(100) as i64).into());
        self.connection()
            .query_all(sql(&query, values))
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
            tx.execute(sql(
                "UPDATE personal_access_tokens SET revoked_at=?,record_json=? WHERE id=?",
                vec![now.into(), json.into(), id.into()],
            ))
            .await
            .map_err(unavailable)?;
            audit(self, &tx, &r, "revoke", now, &actor.authentication).await?;
        }
        tx.commit().await.map_err(unavailable)?;
        Ok(())
    }

    async fn touch(&self, id: &str, now: i64) -> TokenResult<()> {
        self.connection().execute(sql("UPDATE personal_access_tokens SET last_used_at=? WHERE id=? AND (last_used_at IS NULL OR last_used_at<=?)",vec![now.into(),id.into(),now.saturating_sub(60_000).into()])).await.map_err(unavailable)?;
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
