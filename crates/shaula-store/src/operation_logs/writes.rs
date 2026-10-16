use super::{invalid, missing, now, unavailable, OperationLogArchive};
use crate::entities::{
    jobs::workflow_generation_identity as identity,
    lifecycle::runner_generations as generations,
    operation_logs::{
        operation_log_chunks as log_chunks, operation_log_invocations as log_invocations,
    },
};
use async_trait::async_trait;
use sea_orm::{
    sea_query::{Expr, Func},
    ActiveValue::Set,
    ColumnTrait, EntityTrait, JoinType, QueryFilter, QuerySelect,
};
use sha2::{Digest, Sha256};
use shaula_core::{error::CoreResult, operation_log::*};
use std::sync::atomic::Ordering;

#[async_trait]
impl OperationLogSink for OperationLogArchive {
    async fn begin(&self, request: BeginInvocation) -> CoreResult<String> {
        uuid::Uuid::parse_str(&request.generation_id).map_err(|_| invalid())?;
        if !matches!(request.operation.as_str(), "Create" | "Destroy") {
            return Err(invalid());
        }
        let _writer = self.writer.lock().await;
        self.prune_locked(0).await?;
        let (
            fleet_key,
            template_profile_key,
            template_revision,
            template_artifact_digest,
            incarnation,
        ) = generations::Entity::find_by_id(request.generation_id.as_str())
            .select_only()
            .columns([
                generations::Column::FleetKey,
                generations::Column::TemplateProfileKey,
                generations::Column::TemplateRevision,
                generations::Column::TemplateArtifactDigest,
            ])
            .column(identity::Column::FleetIncarnation)
            .join(
                JoinType::LeftJoin,
                generations::Entity::belongs_to(identity::Entity)
                    .from(generations::Column::Id)
                    .to(identity::Column::GenerationId)
                    .into(),
            )
            .into_tuple::<(String, String, i64, String, Option<String>)>()
            .one(self.store.connection())
            .await
            .map_err(|_| unavailable())?
            .ok_or_else(missing)?;
        let ordinal = log_invocations::Entity::find()
            .select_only()
            .expr(
                Expr::expr(Func::coalesce([
                    Expr::col(log_invocations::Column::Ordinal).max(),
                    Expr::val(0).into(),
                ]))
                .add(1),
            )
            .filter(log_invocations::Column::GenerationId.eq(request.generation_id.as_str()))
            .filter(log_invocations::Column::Operation.eq(request.operation.as_str()))
            .into_tuple::<i64>()
            .one(self.store.connection())
            .await
            .map_err(|_| unavailable())?
            .ok_or_else(unavailable)?;
        let record = Invocation {
            id: uuid::Uuid::new_v4().to_string(),
            generation_id: request.generation_id,
            fleet_key,
            fleet_incarnation: incarnation,
            template_profile_key,
            template_revision,
            template_artifact_digest,
            operation: request.operation,
            ordinal,
            effect_attempt_id: None,
            started_at: request.started_at,
            ended_at: None,
            capture_sealed_at: None,
            execution_outcome: "running".into(),
            capture_status: "capturing".into(),
            reason: None,
            content_version: uuid::Uuid::new_v4().to_string(),
            policy_version: SANITIZATION_POLICY.into(),
            retained_bytes: 0,
            lost_bytes: 0,
            commands: Vec::new(),
        };
        let json = serde_json::to_string(&record).map_err(|_| unavailable())?;
        log_invocations::Entity::insert(log_invocations::ActiveModel {
            id: Set(record.id.clone()),
            generation_id: Set(record.generation_id.clone()),
            fleet_key: Set(record.fleet_key.clone()),
            operation: Set(record.operation.clone()),
            ordinal: Set(record.ordinal),
            record_json: Set(json),
            ..Default::default()
        })
        .exec_without_returning(self.store.connection())
        .await
        .map_err(|_| unavailable())?;
        Ok(record.id)
    }

    async fn append(&self, chunk: AppendLog) -> CoreResult<()> {
        if !matches!(chunk.phase.as_str(), "init" | "plan" | "apply")
            || !matches!(chunk.stream.as_str(), "stdout" | "stderr")
            || chunk.text.len() > 65536
        {
            return Err(invalid());
        }
        let sequence = i64::try_from(chunk.sequence).map_err(|_| invalid())?;
        let _writer = self.writer.lock().await;
        let mut record = self.load(&chunk.invocation_id).await?;
        let digest = hex::encode(Sha256::digest(chunk.text.as_bytes()));
        let rows = self.chunk_rows(&record.id).await?;
        if let Some(existing) = rows.iter().find(|r| r.sequence == sequence) {
            return if existing.digest == digest
                && existing.phase == chunk.phase
                && existing.stream == chunk.stream
                && existing.command_ordinal == i64::from(chunk.command_ordinal)
            {
                Ok(())
            } else {
                Err(invalid())
            };
        }
        if record.capture_sealed_at.is_some() {
            return Err(invalid());
        }
        // A replay of an evicted historical chunk must never append it again.
        if rows.last().is_some_and(|r| sequence <= r.sequence) {
            return Err(invalid());
        }
        let bytes = u64::try_from(chunk.text.len()).map_err(|_| invalid())?;
        let head_budget = self.config.invocation_bytes / 2;
        let mut head_used = 0u64;
        for row in &rows {
            if record.retained_bytes.saturating_add(bytes) <= self.config.invocation_bytes {
                break;
            }
            let size = u64::try_from(row.bytes).map_err(|_| unavailable())?;
            if head_used.saturating_add(size) <= head_budget {
                head_used += size;
                continue;
            }
            self.remove_chunk(&record.id, row).await?;
            record.retained_bytes = record.retained_bytes.saturating_sub(size);
            record.lost_bytes = record.lost_bytes.saturating_add(size);
            record.reason = Some("invocation_limit".into());
        }
        self.prune_locked(bytes).await?;
        if self
            .disk_usage
            .load(Ordering::Relaxed)
            .saturating_add(bytes)
            > self.config.quota_bytes
        {
            record.lost_bytes = record.lost_bytes.saturating_add(bytes);
            record.reason = Some("quota_exceeded".into());
            self.save(&record).await?;
            return Ok(());
        }
        // Reserve before a cancellable file operation; failure keeps an upper bound.
        self.disk_usage.fetch_add(bytes, Ordering::Relaxed);
        let name = self
            .publish(&record.id, chunk.sequence, &chunk.text)
            .await?;
        log_chunks::Entity::insert(log_chunks::ActiveModel {
            invocation_id: Set(record.id.clone()),
            sequence: Set(sequence),
            command_ordinal: Set(i64::from(chunk.command_ordinal)),
            phase: Set(chunk.phase),
            stream: Set(chunk.stream),
            observed_at: Set(chunk.observed_at),
            bytes: Set(i64::try_from(bytes).map_err(|_| invalid())?),
            digest: Set(digest),
            file_name: Set(name),
        })
        .exec_without_returning(self.store.connection())
        .await
        .map_err(|_| unavailable())?;
        record.retained_bytes = record.retained_bytes.saturating_add(bytes);
        if chunk.withheld {
            record
                .reason
                .get_or_insert_with(|| "filtered_content".into());
        }
        self.save(&record).await
    }

    async fn command(&self, command: LogCommand) -> CoreResult<()> {
        if !matches!(command.phase.as_str(), "init" | "plan" | "apply")
            || !matches!(
                command.termination.as_str(),
                "running" | "exited" | "not_spawned" | "interrupted" | "reader_error" | "timed_out"
            )
        {
            return Err(invalid());
        }
        let _writer = self.writer.lock().await;
        let mut record = self.load(&command.invocation_id).await?;
        if record.capture_sealed_at.is_some() {
            return Err(invalid());
        }
        if let Some(effect) = &command.effect_attempt_id {
            record.effect_attempt_id = Some(effect.clone());
        }
        let command_count = record.commands.len();
        match record
            .commands
            .iter_mut()
            .find(|c| c.ordinal == command.ordinal)
        {
            Some(previous) if previous.phase == command.phase && previous.ended_at.is_none() => {
                *previous = command
            }
            Some(previous)
                if serde_json::to_string(previous).ok() == serde_json::to_string(&command).ok() =>
            {
                return Ok(())
            }
            Some(_) => return Err(invalid()),
            None if command_count < 64 => record.commands.push(command),
            None => return Err(invalid()),
        }
        self.save(&record).await
    }

    async fn finish(&self, result: FinishInvocation) -> CoreResult<()> {
        if !matches!(
            result.execution_outcome.as_str(),
            "succeeded" | "failed" | "interrupted" | "unknown" | "skipped"
        ) {
            return Err(invalid());
        }
        let _writer = self.writer.lock().await;
        let mut record = self.load(&result.invocation_id).await?;
        if record.capture_sealed_at.is_some() {
            return Ok(());
        }
        record.execution_outcome = result.execution_outcome;
        record.ended_at = Some(result.ended_at);
        record.capture_sealed_at = Some(now());
        record.lost_bytes = record.lost_bytes.saturating_add(result.lost_bytes);
        let incomplete = result.partial || record.lost_bytes > 0;
        record.capture_status = if incomplete { "partial" } else { "complete" }.into();
        if incomplete && record.reason.is_none() {
            record.reason = Some("capture_interrupted".into());
        }
        for command in &mut record.commands {
            if command.ended_at.is_none() {
                command.termination = "interrupted".into();
            }
        }
        self.save(&record).await
    }
}
