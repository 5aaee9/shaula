use super::{invalid, unavailable, OperationLogArchive};
use async_trait::async_trait;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use serde::{Deserialize, Serialize};
use shaula_core::{error::CoreResult, operation_log::*};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    id: String,
    version: String,
    phase: Option<String>,
    stream: Option<String>,
    after: i64,
}

#[async_trait]
impl OperationLogReadPort for OperationLogArchive {
    async fn list_invocations_page(
        &self,
        generation_id: &str,
        query: InvocationQuery,
    ) -> CoreResult<InvocationsPage> {
        self.invocations_page(generation_id, query).await
    }
    async fn list_invocations(&self, generation_id: &str) -> CoreResult<Vec<Invocation>> {
        uuid::Uuid::parse_str(generation_id).map_err(|_| invalid())?;
        let _writer = self.writer.lock().await;
        let rows = self.store.connection().query_all(Statement::from_sql_and_values(DbBackend::Sqlite,
            "SELECT id FROM operation_log_invocations WHERE generation_id=? ORDER BY json_extract(record_json,'$.started_at') DESC,ordinal DESC,id DESC LIMIT 1000", [generation_id.into()]
        )).await.map_err(|_| unavailable())?;
        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            result.push(
                self.load(&row.try_get::<String>("", "id").map_err(|_| unavailable())?)
                    .await?,
            );
        }
        Ok(result)
    }

    async fn read_page(&self, id: &str, query: LogQuery) -> CoreResult<LogPage> {
        if query
            .phase
            .as_deref()
            .is_some_and(|v| !matches!(v, "init" | "plan" | "apply"))
            || query
                .stream
                .as_deref()
                .is_some_and(|v| !matches!(v, "stdout" | "stderr"))
        {
            return Err(invalid());
        }
        let limit = query.limit_bytes.unwrap_or(256 * 1024);
        if !(65536..=1024 * 1024).contains(&limit) {
            return Err(invalid());
        }
        let _writer = self.writer.lock().await;
        let record = self.load(id).await?;
        let mut after = -1;
        if let Some(raw) = &query.cursor {
            if raw.len() > 2048 {
                return Err(invalid());
            }
            let bytes = hex::decode(raw).map_err(|_| invalid())?;
            let cursor: Cursor = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
            if cursor.id != id
                || cursor.version != record.content_version
                || cursor.phase != query.phase
                || cursor.stream != query.stream
                || cursor.after < -1
            {
                return Err(invalid());
            }
            after = cursor.after;
        }
        let mut page = LogPage {
            invocation_id: id.into(),
            content_version: record.content_version.clone(),
            capture_status: record.capture_status.clone(),
            entries: Vec::new(),
            next_cursor: None,
            has_gap: record.lost_bytes > 0,
            lost_bytes: record.lost_bytes,
        };
        if record.policy_version != SANITIZATION_POLICY {
            page.capture_status = "withheld".into();
            return Ok(page);
        }
        if matches!(
            record.capture_status.as_str(),
            "expired" | "unavailable" | "withheld"
        ) {
            return Ok(page);
        }
        let mut used = 0;
        let rows = self.chunk_rows(id).await?;
        let mut more = false;
        for row in rows {
            let sequence: i64 = row.try_get("", "sequence").map_err(|_| unavailable())?;
            if sequence <= after {
                continue;
            }
            let phase: String = row.try_get("", "phase").map_err(|_| unavailable())?;
            let stream: String = row.try_get("", "stream").map_err(|_| unavailable())?;
            if query.phase.as_ref().is_some_and(|value| *value != phase)
                || query.stream.as_ref().is_some_and(|value| *value != stream)
            {
                continue;
            }
            let bytes: i64 = row.try_get("", "bytes").map_err(|_| unavailable())?;
            if used + usize::try_from(bytes).map_err(|_| unavailable())? > limit {
                more = true;
                break;
            }
            let text = match self.chunk_text(id, &row).await {
                Ok(text) => text,
                Err(_) => {
                    page.capture_status = "unavailable".into();
                    page.has_gap = true;
                    page.entries.clear();
                    return Ok(page);
                }
            };
            used += text.len();
            after = sequence;
            page.entries.push(LogEntry {
                command_ordinal: u32::try_from(
                    row.try_get::<i64>("", "command_ordinal")
                        .map_err(|_| unavailable())?,
                )
                .map_err(|_| unavailable())?,
                phase,
                stream,
                sequence: u64::try_from(sequence).map_err(|_| unavailable())?,
                text,
                observed_at: row.try_get("", "observed_at").map_err(|_| unavailable())?,
            });
        }
        // A live cursor remains useful even when no new durable output exists.
        if more || record.capture_sealed_at.is_none() {
            page.next_cursor = Some(hex::encode(
                serde_json::to_vec(&Cursor {
                    id: id.into(),
                    version: record.content_version,
                    phase: query.phase,
                    stream: query.stream,
                    after,
                })
                .map_err(|_| unavailable())?,
            ));
        }
        Ok(page)
    }

    async fn setup_projection(&self, generation_id: &str) -> CoreResult<SetupProjection> {
        let records = self.list_invocations(generation_id).await?;
        let record = records.iter().find(|record| {
            record.operation == "Create" && record.commands.iter().any(|c| c.phase == "apply")
        });
        let Some(record) = record else {
            return Ok(SetupProjection {
                status: if records
                    .iter()
                    .any(|r| r.operation == "Create" && r.capture_sealed_at.is_none())
                {
                    "pending"
                } else {
                    "not_recorded"
                }
                .into(),
                invocation_id: None,
                content_version: None,
                detail: None,
                partial: false,
            });
        };
        let mut projection = SetupProjection {
            status: "pending".into(),
            invocation_id: Some(record.id.clone()),
            content_version: Some(record.content_version.clone()),
            detail: None,
            partial: record.capture_status == "partial" || record.lost_bytes > 0,
        };
        let Some(command) = record
            .commands
            .iter()
            .find(|command| command.phase == "apply")
        else {
            return Ok(projection);
        };
        if command.ended_at.is_none() && record.capture_sealed_at.is_none() {
            return Ok(projection);
        }
        if !matches!(
            record.capture_status.as_str(),
            "complete" | "partial" | "capturing"
        ) {
            projection.status = record.capture_status.clone();
            return Ok(projection);
        }
        let _reader = self.writer.lock().await;
        let record = self.load(&record.id).await?;
        if record.policy_version != SANITIZATION_POLICY {
            projection.status = "withheld".into();
            return Ok(projection);
        }
        if record.capture_status == "expired" {
            projection.status = "expired".into();
            return Ok(projection);
        }
        projection.partial |= command.termination != "exited" || command.capture_partial;
        let mut head = String::new();
        let mut tail = std::collections::VecDeque::new();
        let mut tail_bytes = 0usize;
        let mut filling_head = true;
        for row in self.chunk_rows(&record.id).await? {
            if row
                .try_get::<String>("", "phase")
                .map_err(|_| unavailable())?
                != "apply"
            {
                continue;
            }
            let text = match self.chunk_text(&record.id, &row).await {
                Ok(text) => text,
                Err(_) => {
                    projection.status = "unavailable".into();
                    return Ok(projection);
                }
            };
            for line in text.lines() {
                let line = format!("{}\n", runner_line(line));
                if filling_head && head.len().saturating_add(line.len()) <= 248 * 1024 {
                    head.push_str(&line);
                    continue;
                }
                filling_head = false;
                tail_bytes += line.len();
                tail.push_back(line);
                while tail_bytes > 248 * 1024 {
                    if let Some(discarded) = tail.pop_front() {
                        tail_bytes = tail_bytes.saturating_sub(discarded.len());
                        projection.partial = true;
                    }
                }
            }
        }
        let mut detail = head;
        if projection.partial {
            detail.push_str(
                "[Shaula: provisioning log is incomplete or truncated; retained tail follows]\n",
            );
        }
        for line in tail {
            detail.push_str(&line);
        }
        detail.push_str(&format!(
            "[Shaula: apply process {}; exit code {}]\n",
            command.termination,
            command
                .exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "unknown".into())
        ));
        projection.status = "ready".into();
        projection.detail = Some(detail);
        Ok(projection)
    }
}

/// Workflow readers receive progress without provider IDs or arbitrary diagnostics.
fn runner_line(line: &str) -> String {
    let progress = [
        ": Creating...",
        ": Still creating...",
        ": Creation complete",
        "Apply complete!",
        "[Shaula:",
        "[REDACTED]",
    ];
    if progress.iter().any(|marker| line.contains(marker))
        && !line.contains("::")
        && !line.contains("##[")
    {
        line.split(" [id=").next().unwrap_or_default().to_string()
    } else {
        "[Shaula: diagnostic withheld from workflow audience]".into()
    }
}
