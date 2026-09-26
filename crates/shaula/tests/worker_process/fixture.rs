use sea_orm::{ConnectionTrait, DbBackend, Statement};
use shaula_core::{
    lifecycle::GenerationState as G,
    ports::*,
    registry::{FleetRuntimeGuard, GenerationRecord, LifecycleStore},
    state_backend::StateDocument,
    template::*,
};
use shaula_daemon::{
    apply_intent::LedgerApplyIntentSink,
    effect_gate::FleetEffectGates,
    workers::{WorkerConfig, Workers},
};
use shaula_store::{
    http_state::{SqliteStateBackend, SqliteWorkerJournal, WorkerAdmissions},
    registry_impl::SqliteControlPlane,
    Store,
};
use std::{path::PathBuf, sync::Arc, time::Duration};
use uuid::Uuid;
pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub struct Fixture {
    pub root: tempfile::TempDir,
    pub workspace: PathBuf,
    pub id: Uuid,
    pub backend: SqliteStateBackend,
    pub workers: Arc<Workers>,
    pub executor: Arc<shaula_executor::ExecExecutor>,
    pub faults: Arc<super::faults::StateFaults>,
    pub control_faults: Arc<super::faults::ControlFaults>,
    pub reaper_failures: Arc<std::sync::atomic::AtomicUsize>,
    pub control: Arc<SqliteControlPlane>,
    artifact: PathBuf,
    digest: String,
    db: sea_orm::DatabaseConnection,
    input: ShaulaInputEnvelope,
    sink: Arc<LedgerApplyIntentSink>,
    server: tokio::task::JoinHandle<std::io::Result<()>>,
    stop: tokio::sync::oneshot::Sender<()>,
}

impl Fixture {
    pub async fn new() -> TestResult<Self> {
        Self::with_orphan(false).await
    }
    pub async fn with_orphan(orphan: bool) -> TestResult<Self> {
        let root = tempfile::tempdir()?;
        let work_root = root.path().join("runners");
        let artifact_root = root.path().join("artifacts");
        std::fs::create_dir_all(&work_root)?;
        std::fs::create_dir_all(&artifact_root)?;
        let artifact = artifact_root.join("fixture");
        std::fs::create_dir(&artifact)?;
        let digest = artifact_fixture(&artifact, orphan)?;
        let id = Uuid::new_v4();
        let workspace = work_root.join(id.to_string());
        std::fs::create_dir(&workspace)?;
        let store = Store::open(&root.path().join("state.db")).await?;
        store.migrate().await?;
        let db = sea_orm::Database::connect(format!(
            "sqlite://{}?mode=rw",
            root.path().join("state.db").display()
        ))
        .await?;
        db.execute_unprepared("INSERT INTO fleets(key,incarnation,desired_revision,observed_revision,mutation_fence,deletion_marker,phase,tombstone,created_at,updated_at) VALUES ('fleet','incarnation',1,0,1,0,'Pending',0,1,1)").await?;
        let backend = SqliteStateBackend::new(store.clone());
        backend.activate().await?;
        let admissions = Arc::new(WorkerAdmissions::new(4, 1)?);
        let control = Arc::new(
            SqliteControlPlane::new(store.clone(), artifact_root.clone())
                .with_worker_admissions(admissions.clone()),
        );
        assert!(
            control
                .generation_insert_guarded(
                    GenerationRecord {
                        id: id.to_string(),
                        fleet_key: "fleet".into(),
                        runner_name: "runner".into(),
                        generation_name: format!("s{}", id.simple()),
                        fleet_revision: 1,
                        template_profile_key: "template".into(),
                        pool_member_key: None,
                        template_revision: 1,
                        template_artifact_digest: digest.clone(),
                        attestation_id: "activation".into(),
                        inputs_digest: "fixture".into(),
                        state: G::CreatePending,
                        github_runner_id: None,
                        workspace_path: workspace.to_str().ok_or("workspace encoding")?.into(),
                        created_at: 1,
                        updated_at: 1,
                    },
                    &FleetRuntimeGuard {
                        incarnation: "incarnation".into(),
                        desired_revision: 1,
                        mutation_fence: 1
                    }
                )
                .await?
        );
        control
            .generation_advance(&id.to_string(), G::Creating, 2)
            .await?;
        let journal = Arc::new(SqliteWorkerJournal::new(backend.clone(), admissions));
        let faults = Arc::new(super::faults::StateFaults {
            backend: backend.clone(),
            reject_empty_writes: Default::default(),
            rejected: Default::default(),
        });
        let server =
            shaula_http::state_backend::StateServer::bind("127.0.0.1:0".parse()?, faults.clone())
                .await?;
        let executor = Arc::new(shaula_executor::ExecExecutor::new(
            PathBuf::from(env!("CARGO_BIN_EXE_shaula")),
            PathBuf::from(std::env::var("SHAULA_TEST_CGROUP")?),
        )?);
        let reaper_failures = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let reaper = Arc::new(super::faults::ReaperFaults {
            failures: reaper_failures.clone(),
            inner: shaula_template::cleanup::ReceiptWorkspaceReaper::new(
                work_root.clone(),
                artifact_root.clone(),
            ),
        });
        let workers = Arc::new(
            Workers::new(
                WorkerConfig {
                    max_workers: 4,
                    address: server.local_addr()?,
                    executable_digest: executor.executable_digest().into(),
                    engine: PathBuf::from(std::env::var("SHAULA_TEST_TERRAFORM")?),
                    work_root,
                    artifact_root,
                },
                journal,
                executor.clone(),
            )
            .with_reaper(reaper),
        );
        let control_faults = Arc::new(super::faults::ControlFaults {
            workers: workers.clone(),
            lost_receipts: Default::default(),
        });
        let server = server.with_worker_control(control_faults.clone());
        let (stop, shutdown) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(server.serve(async {
            let _ = shutdown.await;
        }));
        let input = ShaulaInputEnvelope::new(
            GenerationIdentity {
                fleet_key: "fleet".into(),
                scale_set_id: Some(1),
                id: id.to_string(),
                runner_name: "runner".into(),
                generation_name: format!("s{}", id.simple()),
            },
            "protected-fixture-jit".into(),
            BindingsDigest("fixture-commitment".into()),
        );
        let sink = Arc::new(LedgerApplyIntentSink {
            store: control.clone(),
            gates: Arc::new(FleetEffectGates::default()),
        });
        Ok(Self {
            root,
            workspace,
            id,
            backend,
            workers,
            executor,
            faults,
            control_faults,
            reaper_failures,
            control,
            artifact,
            digest,
            db,
            input,
            sink,
            server,
            stop,
        })
    }
    pub fn request(&self) -> TemplateCreateRequest {
        TemplateCreateRequest {
            workspace_path: self.workspace.clone(),
            artifact_dir: self.artifact.clone(),
            pinned_artifact_digest: self.digest.clone(),
            input: self.input.clone(),
            expected_bindings_digest: self.input.bindings_digest.clone(),
            managed_shape: shape(),
            environment: vec![],
            timeout: Duration::from_secs(30),
            apply_intent_sink: Some(self.sink.clone()),
            forgejo_bootstrap: None,
        }
    }
    pub async fn create(&self) -> TestResult<TemplateCreateResult> {
        self.workers
            .prepare_create(
                &self.workspace,
                &self.artifact,
                &self.digest,
                Duration::from_secs(30),
            )
            .await
            .map_err(|e| format!("prepare failed: {e:?}"))?;
        let result = self
            .workers
            .create(self.request())
            .await
            .map_err(|e| format!("create failed: {e:?}"))?;
        // Production supervisors persist this identity only after Create and
        // fixed bootstrap succeed. Recovery must not infer it from state.
        self.control
            .generation_set_result(
                &self.id.to_string(),
                &serde_json::json!({
                    "state_lineage": result.state_lineage, "state_serial": result.state_serial,
                })
                .to_string(),
                "fixture",
                3,
            )
            .await?;
        Ok(result)
    }
    pub async fn destroy(&self, result: &TemplateCreateResult) -> TestResult {
        self.db
            .execute(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "UPDATE runner_generations SET state = 'Destroying' WHERE id = ?",
                vec![self.id.to_string().into()],
            ))
            .await?;
        self.workers
            .destroy(TemplateDestroyRequest {
                workspace_path: self.workspace.clone(),
                artifact_dir: self.artifact.clone(),
                pinned_artifact_digest: self.digest.clone(),
                generation_id: self.id.to_string(),
                expected_bindings_digest: self.input.bindings_digest.clone(),
                managed_shape: shape(),
                environment: vec![],
                timeout: Duration::from_secs(30),
                apply_intent_sink: Some(self.sink.clone()),
                original_provenance: result.provenance.clone(),
                original_state: OriginalStateIdentity {
                    lineage: result.state_lineage.clone(),
                    serial: result.state_serial,
                    allow_serial_advance: true,
                },
            })
            .await
            .map_err(|e| format!("destroy failed: {e:?}"))?;
        Ok(())
    }
    pub async fn state(&self) -> TestResult<StateDocument> {
        let row = self
            .db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "SELECT state_bytes FROM generation_http_state WHERE generation_id = ?",
                vec![self.id.to_string().into()],
            ))
            .await?
            .ok_or("state absent")?;
        Ok(StateDocument::parse(row.try_get("", "state_bytes")?)?)
    }
    pub async fn create_count(&self) -> TestResult<i64> {
        let row = self.db.query_one(Statement::from_sql_and_values(DbBackend::Sqlite, "SELECT COUNT(*) AS count FROM runner_operations WHERE generation_id = ? AND kind = 'Create'", vec![self.id.to_string().into()])).await?.ok_or("count absent")?;
        Ok(row.try_get("", "count")?)
    }
    pub async fn close(self) -> TestResult {
        self.workers.shutdown(Duration::from_secs(1)).await?;
        let _ = self.stop.send(());
        self.server.await??;
        Ok(())
    }
}

fn shape() -> Vec<ManagedResourceRole> {
    vec![ManagedResourceRole {
        role: "runner".into(),
        terraform_type: "terraform_data".into(),
        exact_count: 1,
    }]
}

fn artifact_fixture(path: &std::path::Path, orphan: bool) -> TestResult<String> {
    std::fs::write(
        path.join("profile.yaml"),
        r#"api_version: shaula.io/template-profile/v1
kind: RunnerTemplateProfile
platform: aws
runtime:
  protocol: terraform-cli/v1
  engine: terraform
  root_module: .
  required_version: ">= 1.9, < 2.0"
bindings_contract: shaula.bindings.aws/v1
vm_image_contract: shaula.aws-ami/v1
schemas:
  bindings: schemas/bindings.schema.json
  parameters: schemas/parameters.schema.json
managed_resource_shape:
  - role: runner
    terraform_type: terraform_data
    exact_count: 1
runner_image_digests: []
runtime_policy_digest: sha256:fixture
"#,
    )?;
    std::fs::write(path.join(".terraform.lock.hcl"), "")?;
    std::fs::write(
        path.join("main.tf"),
        r#"terraform { required_version = "= 1.9.8" }
variable "shaula" { type = any }
resource "terraform_data" "runner" { input = var.shaula.generation.id }
output "shaula_result" {
  value = {
    contract_version = 1
    generation_id = var.shaula.generation.id
    bindings_digest = var.shaula.bindings_digest
    resources = [{ role = "runner", id = terraform_data.runner.id }]
  }
}
"#,
    )?;
    if orphan {
        let main = std::fs::read_to_string(path.join("main.tf"))?;
        std::fs::write(
            path.join("main.tf"),
            main.replace(
                "resource \"terraform_data\" \"runner\" { input = var.shaula.generation.id }",
                r#"resource "terraform_data" "runner" {
  input = var.shaula.generation.id
  provisioner "local-exec" {
    command = "setsid sh -c 'echo $$ > orphan.pid; exec sleep 300' </dev/null >/dev/null 2>&1 &"
  }
}"#,
            ),
        )?;
    }
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut archive = tar::Builder::new(encoder);
    for name in ["profile.yaml", ".terraform.lock.hcl", "main.tf"] {
        archive.append_path_with_name(path.join(name), name)?;
    }
    let bytes = archive.into_inner()?.finish()?;
    std::fs::write(path.with_extension("tar.gz"), &bytes)?;
    use sha2::{Digest, Sha256};
    Ok(format!("sha256:{}", hex::encode(Sha256::digest(bytes))))
}
