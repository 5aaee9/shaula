//! Exact execution references and immutable context retention.
use crate::store::{Store, StoreError, StoreResult};
use sea_orm::{ConnectionTrait, DatabaseTransaction, DbBackend, Statement};
use shaula_core::auth_context::ResolvedAuthContext;

pub(crate) struct ExecutionDependency {
    pub target_json: String,
    pub reference: (String, i64),
    pub context: Option<ResolvedAuthContext>,
}

impl Store {
    pub(crate) async fn auth_archive_context_tx(
        &self,
        tx: &DatabaseTransaction,
        fleet: &str,
        context: &ResolvedAuthContext,
    ) -> StoreResult<()> {
        let json =
            serde_json::to_string(context).map_err(|e| StoreError::Corrupt(e.to_string()))?;
        let previous = self
            .auth_execution_context_tx(tx, fleet, &context.profile_key, context.revision)
            .await?;
        if previous.as_ref().is_some_and(|old| old != context) {
            return Err(StoreError::Corrupt(
                "attempt to replace retained execution context".into(),
            ));
        }
        tx.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
            "INSERT OR IGNORE INTO fleet_auth_context_history(fleet_key,profile_key,revision,context_json) VALUES(?,?,?,?)",
            [fleet.into(), context.profile_key.clone().into(), context.revision.into(), json.into()])).await?;
        Ok(())
    }

    pub(crate) async fn auth_execution_context_tx(
        &self,
        tx: &DatabaseTransaction,
        fleet: &str,
        key: &str,
        revision: i64,
    ) -> StoreResult<Option<ResolvedAuthContext>> {
        let row = tx.query_one(Statement::from_sql_and_values(DbBackend::Sqlite,
            "SELECT context_json FROM fleet_auth_context_history WHERE fleet_key=? AND profile_key=? AND revision=?",
            [fleet.into(),key.into(),revision.into()])).await?;
        row.map(|row| {
            let json: String = row.try_get("", "context_json")?;
            let context: ResolvedAuthContext = serde_json::from_str(&json)
                .map_err(|e| StoreError::Corrupt(format!("retained auth context corrupt: {e}")))?;
            if context.profile_key != key
                || context.revision != revision
                || !context.has_complete_identity()
            {
                return Err(StoreError::Corrupt(
                    "retained auth context does not match its reference".into(),
                ));
            }
            Ok(context)
        })
        .transpose()
    }

    pub(crate) async fn auth_execution_context_get(
        &self,
        fleet: &str,
        key: &str,
        revision: i64,
    ) -> StoreResult<Option<ResolvedAuthContext>> {
        let tx = self.begin().await?;
        let row = self
            .auth_revision_get_tx(&tx, key, revision)
            .await?
            .ok_or_else(|| StoreError::Corrupt("execution auth revision missing".into()))?;
        if row.schema_version != 2 || row.kind != "github_app" {
            return Err(StoreError::PolicyDenied {
                reason: "UnsupportedAuthFormat",
            });
        }
        let result = self
            .auth_execution_context_tx(&tx, fleet, key, revision)
            .await?;
        tx.commit().await?;
        Ok(result)
    }

    pub(crate) async fn auth_generation_ref_tx(
        &self,
        tx: &DatabaseTransaction,
        generation: &str,
    ) -> StoreResult<Option<(String, i64)>> {
        let row = tx.query_one(Statement::from_sql_and_values(DbBackend::Sqlite,
            "SELECT o.provenance_json FROM runner_operations o WHERE o.generation_id=? AND o.kind='JitStarting' ORDER BY o.created_at LIMIT 1",
            [generation.into()])).await?;
        if let Some(row) = row {
            let json: Option<String> = row.try_get("", "provenance_json")?;
            if let Some(json) = json {
                let value: serde_json::Value = serde_json::from_str(&json).map_err(|e| {
                    StoreError::Corrupt(format!("JIT auth provenance corrupt: {e}"))
                })?;
                if let Some(key) = value.pointer("/context/auth_profile_key") {
                    let key = key
                        .as_str()
                        .ok_or_else(|| StoreError::Corrupt("JIT auth key invalid".into()))?;
                    let revision = value
                        .pointer("/context/auth_revision")
                        .and_then(|v| v.as_i64())
                        .ok_or_else(|| StoreError::Corrupt("JIT auth revision missing".into()))?;
                    return Ok(Some((key.to_string(), revision)));
                }
            }
        }
        let row = tx.query_one(Statement::from_sql_and_values(DbBackend::Sqlite,
            "SELECT r.auth_desired_profile_key AS profile_key, r.auth_desired_revision AS revision
             FROM runner_generations g JOIN fleet_revisions r ON r.fleet_key=g.fleet_key AND r.revision=g.fleet_revision WHERE g.id=?",
            [generation.into()])).await?;
        row.map(|r| Ok((r.try_get("", "profile_key")?, r.try_get("", "revision")?)))
            .transpose()
    }

    pub(crate) async fn auth_generation_ref(
        &self,
        generation: &str,
    ) -> StoreResult<Option<(String, i64)>> {
        let tx = self.begin().await?;
        let result = self.auth_generation_ref_tx(&tx, generation).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub(crate) async fn auth_execution_dependencies_tx(
        &self,
        tx: &DatabaseTransaction,
        fleet: &str,
        profile: Option<&str>,
    ) -> StoreResult<Vec<ExecutionDependency>> {
        let mut references = Vec::new();
        let rows = tx.query_all(Statement::from_sql_and_values(DbBackend::Sqlite,
            "SELECT h.observed_profile_key AS profile_key, h.observed_revision AS revision, r.spec_json
             FROM fleet_auth_handoffs h JOIN fleets f ON f.key=h.fleet_key
             JOIN fleet_revisions r ON r.fleet_key=f.key AND r.revision=f.desired_revision
             WHERE h.fleet_key=? AND h.observed_profile_key IS NOT NULL
             UNION ALL SELECT a.profile_key,a.revision,r.spec_json FROM fleet_session_auth a
             JOIN fleet_sessions s ON s.fleet_key=a.fleet_key JOIN fleets f ON f.key=a.fleet_key
             JOIN fleet_revisions r ON r.fleet_key=f.key AND r.revision=f.desired_revision WHERE a.fleet_key=?
             UNION ALL SELECT m.profile_key,m.auth_revision,r.spec_json FROM listener_messages m
             JOIN fleet_revisions r ON r.fleet_key=m.fleet_key AND r.revision=m.fleet_revision
             WHERE m.fleet_key=? AND EXISTS(SELECT 1 FROM listener_acquisitions a
                WHERE a.fleet_key=m.fleet_key AND a.epoch=m.epoch AND a.message_id=m.message_id
                AND a.state IN ('Pending','AcquireStarting','Uncertain'))",
            [fleet.into(),fleet.into(),fleet.into()])).await?;
        for row in rows {
            references.push((
                (
                    row.try_get("", "profile_key")?,
                    row.try_get("", "revision")?,
                ),
                row.try_get::<String>("", "spec_json")?,
            ));
        }
        let generations = tx.query_all(Statement::from_sql_and_values(DbBackend::Sqlite,
            "SELECT g.id,r.spec_json FROM runner_generations g JOIN fleet_revisions r ON r.fleet_key=g.fleet_key AND r.revision=g.fleet_revision
             WHERE g.fleet_key=? AND (g.state!='Destroyed' OR EXISTS (
                SELECT 1 FROM runner_operations o WHERE o.generation_id=g.id
                AND o.state IN ('Pending','Starting','ApplyStarting','BootstrapStarting','Running','Blocked')))", [fleet.into()])).await?;
        for row in generations {
            let id: String = row.try_get("", "id")?;
            let reference = self
                .auth_generation_ref_tx(tx, &id)
                .await?
                .ok_or_else(|| StoreError::Corrupt("generation auth reference missing".into()))?;
            references.push((reference, row.try_get("", "spec_json")?));
        }
        let mut result = Vec::new();
        for (reference, spec) in references {
            if profile.is_some_and(|key| key != reference.0) {
                continue;
            }
            let revision = self
                .auth_revision_get_tx(tx, &reference.0, reference.1)
                .await?
                .ok_or_else(|| StoreError::Corrupt("execution auth revision missing".into()))?;
            // Inventory must retain unsupported references for recovery. It
            // never loads their policy, credential or context as authority.
            let (target_json, context) = if revision.schema_version == 2
                && revision.kind == "github_app"
            {
                let context = self
                    .auth_execution_context_tx(tx, fleet, &reference.0, reference.1)
                    .await?
                    .ok_or_else(|| StoreError::Corrupt("v2 execution context missing".into()))?;
                (
                    serde_json::to_string(&context.target)
                        .map_err(|e| StoreError::Corrupt(e.to_string()))?,
                    Some(context),
                )
            } else {
                (target_from_spec(&spec)?, None)
            };
            result.push(ExecutionDependency {
                target_json,
                reference,
                context,
            });
        }
        Ok(result)
    }

    pub(crate) async fn auth_execution_refs(&self, fleet: &str) -> StoreResult<Vec<(String, i64)>> {
        let tx = self.begin().await?;
        let mut refs: Vec<_> = self
            .auth_execution_dependencies_tx(&tx, fleet, None)
            .await?
            .into_iter()
            .map(|d| d.reference)
            .collect();
        refs.sort();
        refs.dedup();
        tx.commit().await?;
        Ok(refs)
    }
}

pub(crate) fn target_from_spec(spec: &str) -> StoreResult<String> {
    let spec: serde_json::Value =
        serde_json::from_str(spec).map_err(|e| StoreError::Corrupt(e.to_string()))?;
    let target: shaula_core::github::GitHubTarget =
        serde_json::from_value(spec["github"]["target"].clone())
            .map_err(|e| StoreError::Corrupt(format!("fleet target corrupt: {e}")))?;
    serde_json::to_string(&target).map_err(|e| StoreError::Corrupt(e.to_string()))
}
