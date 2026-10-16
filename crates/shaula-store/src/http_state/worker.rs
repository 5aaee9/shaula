//! Worker journal primitives. All checks run after reserving SQLite's writer.
use sea_orm::{
    sea_query::Expr, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseTransaction,
    EntityTrait, FromQueryResult, QueryFilter, QuerySelect,
};
use shaula_core::{
    state_backend::{StateDocument, StateError, StateResult},
    worker::{ControlAccess, ProcessIdentity, WorkerAdmission},
};
use uuid::Uuid;

use super::{exactly_one, row::Row, states, unavailable, workers, SqliteStateBackend};
use crate::entities::lifecycle::{
    forgejo_runner_identities as forgejo_identities, runner_generations as generations,
    runner_operations as operations,
};
use crate::query_ext::{probe, same};

#[derive(FromQueryResult)]
pub(super) struct WorkerRow {
    control_hash: Vec<u8>,
    pub phase: String,
    pub process_identity: Option<String>,
    pub cleanup_only: bool,
    pub handover: Option<String>,
    pub active_command: Option<String>,
    pub cleanup_revision: Option<i64>,
    pub completion_request: Option<String>,
    pub completion_receipt: Option<String>,
}

impl WorkerRow {
    pub async fn read(conn: &impl ConnectionTrait, id: &str) -> StateResult<Option<Self>> {
        workers::Entity::find_by_id(id)
            .select_only()
            .columns([
                workers::Column::ControlHash,
                workers::Column::Phase,
                workers::Column::ProcessIdentity,
                workers::Column::CleanupOnly,
                workers::Column::Handover,
                workers::Column::ActiveCommand,
                workers::Column::CleanupRevision,
                workers::Column::CompletionRequest,
                workers::Column::CompletionReceipt,
            ])
            .into_model::<Self>()
            .one(conn)
            .await
            .map_err(unavailable)
    }

    /// Retained protected input, read apart from the row: it may be large.
    pub async fn protected_input(
        conn: &impl ConnectionTrait,
        id: &str,
    ) -> StateResult<Option<Vec<u8>>> {
        workers::Entity::find_by_id(id)
            .select_only()
            .column(workers::Column::ProtectedInput)
            .into_tuple::<Option<Vec<u8>>>()
            .one(conn)
            .await
            .map_err(unavailable)?
            .ok_or(StateError::Unavailable)
    }

    pub async fn load(tx: &impl ConnectionTrait, access: &ControlAccess) -> StateResult<Self> {
        let state = Row::load(tx, &access.claim.generation_id.to_string()).await?;
        state
            .check_claim(&access.claim)
            .map_err(|_| StateError::Unauthorized)?;
        let worker = Self::read(tx, &access.claim.generation_id.to_string())
            .await?
            .ok_or(StateError::Unauthorized)?;
        if state.revoked || !access.capability.matches(&worker.control_hash) {
            return Err(StateError::Unauthorized);
        }
        Ok(worker)
    }
}

impl SqliteStateBackend {
    /// Composes with the SAME Generation/pool admission transaction. Existing
    /// rows, any CI side effect, or any previous backend are never initialized.
    pub(crate) async fn admit_worker_on(
        tx: &DatabaseTransaction,
        admission: &WorkerAdmission,
    ) -> StateResult<()> {
        if admission.claim.worker_epoch != 1 {
            return Err(StateError::Invalid);
        }
        let id = admission.claim.generation_id.to_string();
        let operation = probe(operations::Entity)
            .and_where(same(
                operations::Column::GenerationId,
                generations::Column::Id,
            ))
            .to_owned();
        let forgejo = probe(forgejo_identities::Entity)
            .and_where(same(
                forgejo_identities::Column::GenerationId,
                generations::Column::Id,
            ))
            .to_owned();
        let (state, jit_phase, github_runner_id) = generations::Entity::find_by_id(&id)
            .select_only()
            .columns([
                generations::Column::State,
                generations::Column::JitPhase,
                generations::Column::GithubRunnerId,
            ])
            .filter(Expr::exists(operation).not())
            .filter(Expr::exists(forgejo).not())
            .into_tuple::<(String, Option<String>, Option<i64>)>()
            .one(tx)
            .await
            .map_err(unavailable)?
            .ok_or(StateError::Conflict)?;
        if state != "CreatePending" || jit_phase.is_some() || github_runner_id.is_some() {
            return Err(StateError::Conflict);
        }
        // Only a proven fresh admission can manufacture serial zero. init is
        // not required to POST an empty snapshot for the Create gate to work.
        let document = StateDocument::parse(
            serde_json::to_vec(&serde_json::json!({
                "version": 4, "serial": 0, "lineage": Uuid::new_v4().to_string(),
                "outputs": {}, "resources": [],
            }))
            .map_err(|_| StateError::Invalid)?,
        )?;
        states::Entity::insert(states::ActiveModel {
            generation_id: Set(id.clone()),
            worker_epoch: Set(admission.claim.worker_epoch),
            worker_attempt: Set(admission.claim.worker_attempt.to_string()),
            capability_hash: Set(admission.state.verifier()),
            revision: Set(1),
            state_bytes: Set(Some(document.bytes().to_vec())),
            lineage: Set(Some(document.lineage().to_owned())),
            serial: Set(Some(0)),
            ..Default::default()
        })
        .exec_without_returning(tx)
        .await
        .map_err(unavailable)?;
        workers::Entity::insert(workers::ActiveModel {
            generation_id: Set(id),
            control_hash: Set(admission.control.verifier()),
            phase: Set("admitted".into()),
            ..Default::default()
        })
        .exec_without_returning(tx)
        .await
        .map_err(unavailable)?;
        Ok(())
    }

    pub async fn authenticate_worker(&self, access: &ControlAccess) -> StateResult<()> {
        WorkerRow::load(self.store.connection(), access).await?;
        Ok(())
    }

    /// Persist before calling exec. A second launcher cannot borrow a pending
    /// launch, even when no process is present in its in-memory child map.
    pub async fn worker_launch_pending(&self, access: &ControlAccess) -> StateResult<()> {
        let id = access.claim.generation_id.to_string();
        let tx = self.writer(&id).await?;
        let worker = WorkerRow::load(&tx, access).await?;
        if worker.phase != "admitted" {
            return Err(StateError::Conflict);
        }
        let pending = workers::Entity::update_many()
            .col_expr(workers::Column::Phase, Expr::value("launch_pending"))
            .filter(workers::Column::GenerationId.eq(id))
            .filter(workers::Column::Phase.eq("admitted"))
            .exec(&tx)
            .await
            .map_err(unavailable)?;
        exactly_one(pending.rows_affected)?;
        tx.commit().await.map_err(unavailable)
    }

    /// Executor-owned identity, not an arbitrary PID supplied by the Worker.
    /// This method is not exposed directly as a Worker control route.
    pub async fn worker_register(
        &self,
        access: &ControlAccess,
        identity: &ProcessIdentity,
    ) -> StateResult<()> {
        identity.validate()?;
        let identity = serde_json::to_string(identity).map_err(|_| StateError::Invalid)?;
        let id = access.claim.generation_id.to_string();
        let tx = self.writer(&id).await?;
        let worker = WorkerRow::load(&tx, access).await?;
        if worker.phase == "running" && worker.process_identity.as_deref() == Some(&identity) {
            return Ok(());
        }
        if worker.phase != "launch_pending" || worker.process_identity.is_some() {
            return Err(StateError::Conflict);
        }
        workers::Entity::update_many()
            .col_expr(workers::Column::Phase, Expr::value("running"))
            .col_expr(workers::Column::ProcessIdentity, Expr::value(identity))
            .filter(workers::Column::GenerationId.eq(id))
            .exec(&tx)
            .await
            .map_err(unavailable)?;
        tx.commit().await.map_err(unavailable)
    }
}
