//! Level-triggered, restart-safe logical retirement. No protected bytes are GC'd.

use super::{core_err, retirement_refs, SqliteControlPlane};
use crate::store::StoreResult;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use shaula_core::error::CoreResult;

impl SqliteControlPlane {
    pub(super) async fn scan_retirements(&self, now: i64) -> CoreResult<usize> {
        let mut completed = 0;
        for template in [true, false] {
            let table = if template {
                "template_profiles"
            } else {
                "github_auth_profiles"
            };
            // Only keys are captured outside the transaction. All authority and
            // references are re-read after reserving the writer.
            let rows = self
                .store
                .connection()
                .query_all(Statement::from_string(
                    DbBackend::Sqlite,
                    format!(
                        "SELECT key FROM {table} WHERE deletion_requested=1 AND status='Retiring'"
                    ),
                ))
                .await
                .map_err(|e| core_err(e.into()))?;
            for row in rows {
                let key: String = row.try_get("", "key").map_err(|e| core_err(e.into()))?;
                completed += usize::from(
                    self.finish_retirement(&key, template, now)
                        .await
                        .map_err(core_err)?,
                );
            }
        }
        Ok(completed)
    }

    async fn finish_retirement(&self, key: &str, template: bool, now: i64) -> StoreResult<bool> {
        let tx = self.store.begin().await?;
        let (table, kind, attestation) = if template {
            (
                "template_profiles",
                "template_profile",
                ", active_attestation_id=NULL",
            )
        } else {
            ("github_auth_profiles", "github_auth_profile", "")
        };
        let head = tx.query_one(Statement::from_sql_and_values(DbBackend::Sqlite,
            format!("SELECT desired_revision FROM {table} WHERE key=? AND deletion_requested=1 AND status='Retiring'"),
            [key.into()],
        )).await?;
        let Some(head) = head else { return Ok(false) };
        if retirement_refs::in_use(&tx, key, template, now).await? {
            return Ok(false);
        }
        let revision: i64 = head.try_get("", "desired_revision")?;
        // desired_revision remains only a historical high-water mark, never an
        // execution head: deletion_requested fences PUT/activation/references.
        tx.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
            format!("UPDATE {table} SET status='Retired',active_revision=NULL,observed_revision=NULL{attestation},updated_at=? WHERE key=?"),
            [now.into(),key.into()],
        )).await?;
        tx.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
            "UPDATE profile_changes SET state=CASE WHEN kind='Retire' THEN 'Succeeded' ELSE 'Superseded' END,
                reason=NULL,next_retry_at=NULL,lease_owner=NULL,lease_expires_at=NULL,updated_at=?
             WHERE resource_kind=? AND profile_key=? AND revision<=? AND state IN ('Pending','Running','Blocked')",
            [now.into(),kind.into(),key.into(),revision.into()],
        )).await?;
        self.store
            .audit_append(
                &tx,
                shaula_core::registry::AuditAppend {
                    resource_kind: kind.into(),
                    action: "retire".into(),
                    actor: "reconciler".into(),
                    resource_key: key.into(),
                    revision: Some(revision),
                    outcome: "succeeded".into(),
                    detail_json: None,
                    now,
                },
            )
            .await?;
        tx.commit().await?;
        Ok(true)
    }
}
