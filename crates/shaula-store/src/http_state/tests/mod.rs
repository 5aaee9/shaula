use std::path::PathBuf;

use sea_orm::ConnectionTrait;
use shaula_core::{
    lifecycle::GenerationState,
    registry::GenerationRecord,
    state_backend::{LockInfo, StateAccess, StateClaim, StateDocument, StateResult},
};
use tempfile::TempDir;
use uuid::Uuid;

use super::{sql, SqliteStateBackend};
use crate::Store;

mod cas;
mod migration;
mod recovery;

pub(super) type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub(super) struct Fixture {
    _dir: TempDir,
    path: PathBuf,
    store: Store,
    backend: SqliteStateBackend,
    claim: StateClaim,
    access: StateAccess,
}

impl Fixture {
    async fn new() -> TestResult<Self> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("state.db");
        let store = Store::open(&path).await?;
        store.migrate().await?;
        store
            .connection()
            .execute_unprepared(
                "INSERT INTO fleets (key, incarnation, desired_revision, observed_revision,
                mutation_fence, deletion_marker, phase, tombstone, created_at, updated_at)
             VALUES ('fleet', 'incarnation', 1, 0, 1, 0, 'Pending', 0, 1, 1)",
            )
            .await?;
        let backend = SqliteStateBackend::new(store.clone());
        let id = Uuid::new_v4();
        let (claim, capability) = backend
            .insert_generation(record(id), Uuid::new_v4())
            .await?;
        let access = StateAccess {
            generation_id: id,
            capability,
        };
        Ok(Self {
            _dir: dir,
            path,
            store,
            backend,
            claim,
            access,
        })
    }

    async fn second_backend(&self) -> TestResult<SqliteStateBackend> {
        Ok(SqliteStateBackend::new(Store::open(&self.path).await?))
    }

    async fn corrupt(&self, assignments: &str) -> TestResult {
        self.store
            .connection()
            .execute(sql(
                &format!("UPDATE generation_http_state SET {assignments} WHERE generation_id = ?"),
                vec![self.claim.generation_id.to_string().into()],
            ))
            .await?;
        Ok(())
    }
}

fn record(id: Uuid) -> GenerationRecord {
    GenerationRecord {
        id: id.to_string(),
        fleet_key: "fleet".into(),
        runner_name: format!("runner-{id}"),
        generation_name: format!("s{}", id.simple()),
        fleet_revision: 1,
        template_profile_key: "template".into(),
        template_revision: 1,
        template_artifact_digest: "sha256:artifact".into(),
        attestation_id: "attested".into(),
        inputs_digest: "sha256:inputs".into(),
        state: GenerationState::CreatePending,
        github_runner_id: None,
        workspace_path: "protected/workspace".into(),
        created_at: 1,
        updated_at: 1,
    }
}

fn lock(id: &str) -> StateResult<LockInfo> {
    LockInfo::parse(
        serde_json::json!({"ID": id, "Operation": "OperationTypeApply"})
            .to_string()
            .as_bytes(),
    )
}

fn state(serial: i64, lineage: &str, live: bool) -> StateResult<StateDocument> {
    let resources = if live {
        serde_json::json!([{
            "mode":"managed", "type":"terraform_data", "name":"runner",
            "provider":"provider[\"terraform.io/builtin/terraform\"]", "instances":[{"attributes":{"id":"protected"}}]
        }])
    } else {
        serde_json::json!([])
    };
    StateDocument::parse(
        serde_json::json!({
            "version":4, "terraform_version":"1.9.8", "lineage":lineage, "serial":serial,
            "outputs":{}, "resources":resources,
        })
        .to_string()
        .into_bytes(),
    )
}
