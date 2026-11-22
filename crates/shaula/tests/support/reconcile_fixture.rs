use std::collections::VecDeque;
use std::path::Path;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

use sea_orm::ConnectionTrait;
use shaula_core::lifecycle::GenerationState as G;
use shaula_core::plan::PlanIntent;
use shaula_core::ports::{
    DestroyClassification, PlanProvenance, ReconcileClassification, StateLineage,
    TemplateCreateRequest, TemplateCreateResult, TemplateDestroyRequest, TemplateOutcomeError,
    TemplateReconcileRequest, TemplateRuntimePort,
};
use shaula_core::registry::{FleetStore, GenerationRecord, LifecycleStore};
use shaula_core::state_backend::{LockInfo, StateAccess, StateBackend, StateDocument};
use shaula_core::worker::journal::WorkerJournal;
use shaula_core::worker::{ControlAccess, FenceOutcome, ProcessIdentity};
use shaula_store::http_state::{SqliteStateBackend, SqliteWorkerJournal, WorkerAdmissions};
use shaula_store::registry_impl::SqliteControlPlane;
use uuid::Uuid;

pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub struct ScriptedRuntime {
    pub reconcile_calls: AtomicUsize,
    pub destroy_calls: AtomicUsize,
    pub reconcile_responses: Mutex<VecDeque<Result<ReconcileClassification, TemplateOutcomeError>>>,
    pub destroy_db: Mutex<Option<(sea_orm::DatabaseConnection, String, String)>>,
}

impl ScriptedRuntime {
    pub fn new(
        reconcile_responses: Vec<Result<ReconcileClassification, TemplateOutcomeError>>,
    ) -> Self {
        Self {
            reconcile_calls: AtomicUsize::new(0),
            destroy_calls: AtomicUsize::new(0),
            reconcile_responses: Mutex::new(reconcile_responses.into()),
            destroy_db: Mutex::new(None),
        }
    }
}

#[async_trait::async_trait]
impl TemplateRuntimePort for ScriptedRuntime {
    async fn create(
        &self,
        _: TemplateCreateRequest,
    ) -> Result<TemplateCreateResult, TemplateOutcomeError> {
        Err(TemplateOutcomeError::PlanFailed {
            phase: "unexpected.create".into(),
        })
    }

    async fn destroy(
        &self,
        _: TemplateDestroyRequest,
    ) -> Result<DestroyClassification, TemplateOutcomeError> {
        self.destroy_calls.fetch_add(1, Ordering::SeqCst);
        let hook = self.destroy_db.lock().ok().and_then(|guard| guard.clone());
        if let Some((db, id, lineage)) = hook {
            if let Ok(empty) = state_doc(&lineage, 2, &[]) {
                let bytes = empty.bytes().to_vec();
                let _ = db
                    .execute(sea_orm::Statement::from_sql_and_values(
                        sea_orm::DbBackend::Sqlite,
                        "UPDATE generation_http_state SET state_bytes = ?, serial = 2, revision = revision + 1 WHERE generation_id = ?",
                        vec![bytes.into(), id.clone().into()],
                    ))
                    .await;
                let _ = db
                    .execute(sea_orm::Statement::from_sql_and_values(
                        sea_orm::DbBackend::Sqlite,
                        "UPDATE lifecycle_workers SET cleanup_revision = (SELECT revision FROM generation_http_state WHERE generation_id = ?) WHERE generation_id = ?",
                        vec![id.clone().into(), id.into()],
                    ))
                    .await;
            }
        }
        Ok(DestroyClassification::Applied)
    }

    async fn reconcile(
        &self,
        _: TemplateReconcileRequest,
    ) -> Result<ReconcileClassification, TemplateOutcomeError> {
        self.reconcile_calls.fetch_add(1, Ordering::SeqCst);
        self.reconcile_responses
            .lock()
            .map_err(|_| TemplateOutcomeError::StateUnavailable {
                phase: "reconcile.lock".into(),
            })?
            .pop_front()
            .unwrap_or_else(|| {
                Err(TemplateOutcomeError::PlanFailed {
                    phase: "reconcile.exhausted".into(),
                })
            })
    }
}

pub fn make_reconcile_artifact(
    dir: &Path,
    settle_delay_secs: u64,
    with_reconcile: bool,
) -> TestResult<(String, Vec<u8>)> {
    let reconcile_yaml = if with_reconcile {
        format!(
            "reconcile:\n  discover_module: reconcile\n  settle_delay_secs: {settle_delay_secs}\n"
        )
    } else {
        String::new()
    };
    let manifest = format!(
        r#"api_version: shaula.io/template-profile/v1
kind: RunnerTemplateProfile
platform: proxmox
vm_image_contract: shaula.proxmox-template/v1
runtime:
  protocol: terraform-cli/v1
  engine: terraform
  root_module: .
  required_version: ">= 1.9, < 2.0"
bindings_contract: shaula.bindings.proxmox/v1
schemas:
  bindings: schemas/bindings.schema.json
  parameters: schemas/parameters.schema.json
managed_resource_shape:
  - role: bootstrap
    terraform_type: proxmox_nocloud_iso
    exact_count: 1
  - role: runner
    terraform_type: proxmox_qemu_vm
    exact_count: 1
runner_image_digests: []
runtime_policy_digest: sha256:policy-v1
{reconcile_yaml}"#
    );
    let mut files: Vec<(&str, Vec<u8>)> = vec![
        ("profile.yaml", manifest.into_bytes()),
        (".terraform.lock.hcl", b"# lock\n".to_vec()),
        ("main.tf", b"variable \"shaula\" { type = any }\n".to_vec()),
        ("schemas/bindings.schema.json", b"{}".to_vec()),
        ("schemas/parameters.schema.json", b"{}".to_vec()),
    ];
    if with_reconcile {
        files.push((
            "reconcile/main.tf",
            b"variable \"shaula\" {\n  type = any\n}\noutput \"shaula_discover\" {\n  value = { contract_version = 1, generation_id = var.shaula.generation.id, roles = [] }\n}\n"
                .to_vec(),
        ));
    }
    let mut builder = tar::Builder::new(Vec::new());
    for (name, content) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append_data(&mut header, name, content.as_slice())?;
    }
    let tar_bytes = builder.into_inner()?;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    std::io::Write::write_all(&mut encoder, &tar_bytes)?;
    let bytes = encoder.finish()?;
    use sha2::Digest as _;
    let digest = format!("sha256:{}", hex::encode(sha2::Sha256::digest(&bytes)));
    shaula_template::ArtifactStore::new(dir).publish(&bytes, &digest)?;
    Ok((digest, bytes))
}

pub fn state_doc(
    lineage: &str,
    serial: i64,
    resources: &[(&str, &str)],
) -> TestResult<StateDocument> {
    let entries: Vec<serde_json::Value> = resources
        .iter()
        .map(|(ty, name)| {
            serde_json::json!({
                "mode": "managed",
                "type": ty,
                "name": name,
                "provider": "provider[\"registry.terraform.io/indexyz/proxmox\"]",
                "instances": [{"attributes": {"id": name}}]
            })
        })
        .collect();
    let bytes = serde_json::to_vec(&serde_json::json!({
        "version": 4,
        "lineage": lineage,
        "serial": serial,
        "outputs": {},
        "resources": entries
    }))?;
    Ok(StateDocument::parse(bytes)?)
}

pub struct SeededFailedCreate {
    pub id: Uuid,
    pub lineage: String,
    pub serial: u64,
}

pub struct SeedOpts<'a> {
    pub profile_key: &'a str,
    pub digest: &'a str,
    pub resources: &'a [(&'a str, &'a str)],
    pub leave_locked: bool,
    pub recover_fenced: bool,
    pub now: i64,
}

pub async fn seed_failed_create(
    control: &SqliteControlPlane,
    backend: &SqliteStateBackend,
    admissions: &Arc<WorkerAdmissions>,
    opts: SeedOpts<'_>,
) -> TestResult<SeededFailedCreate> {
    let id = Uuid::new_v4();
    let id_str = id.to_string();
    let head = control.fleet_get("f1").await?.ok_or("fleet missing")?;
    let guard = shaula_core::registry::FleetRuntimeGuard::from(&head);
    let record = GenerationRecord {
        id: id_str.clone(),
        fleet_key: "f1".into(),
        runner_name: format!("runner-{id_str}"),
        generation_name: format!("gen-{id_str}"),
        fleet_revision: head.desired_revision,
        pool_member_key: None,
        template_profile_key: opts.profile_key.into(),
        template_revision: 1,
        template_artifact_digest: opts.digest.into(),
        attestation_id: "att".into(),
        inputs_digest: "inputs".into(),
        state: G::CreatePending,
        github_runner_id: None,
        workspace_path: "unused".into(),
        created_at: opts.now,
        updated_at: opts.now,
    };
    assert!(control.generation_insert_guarded(record, &guard).await?);
    control
        .generation_set_github_runner(&id_str, 7001, opts.now)
        .await?;
    let admission = admissions.take(id)?;
    let state_access = StateAccess {
        generation_id: id,
        capability: admission.state,
    };
    let control_access = ControlAccess {
        claim: admission.claim.clone(),
        capability: admission.control,
    };
    backend.worker_launch_pending(&control_access).await?;
    let process = ProcessIdentity {
        host_boot: "host:boot".into(),
        process_id: 9001,
        started: "100".into(),
        containment: format!("/cgroup/{id_str}"),
        root_id: None,
    };
    backend.worker_register(&control_access, &process).await?;
    let journal = SqliteWorkerJournal::new(backend.clone(), admissions.clone());
    let input_bytes = b"{\"shaula\":{}}";
    journal
        .retain_input(&control_access, input_bytes.to_vec())
        .await?;
    use sha2::Digest as _;
    let input_digest = format!("sha256:{}", hex::encode(sha2::Sha256::digest(input_bytes)));
    let provenance = PlanProvenance {
        intent: PlanIntent::Create,
        saved_plan_digest: "sha256:plan".into(),
        engine_kind: "terraform".into(),
        engine_version: "1.9.8".into(),
        engine_binary_digest: "sha256:engine".into(),
        artifact_digest: opts.digest.into(),
        template_material_digest: "sha256:material".into(),
        protected_input_digest: input_digest,
        state_lineage: StateLineage::Empty,
        generation_id: id_str.clone(),
        attempt_id: Uuid::new_v4().to_string(),
    };
    control
        .operation_record_apply_starting(&provenance, "tfplan", opts.now)
        .await?;
    journal
        .spawn_ack(&control_access, &provenance.attempt_id)
        .await?;
    journal
        .command_ended(&control_access, &provenance.attempt_id)
        .await?;

    let snap = backend
        .read(&state_access)
        .await?
        .ok_or("initial state missing")?;
    let lineage = snap.document.lineage().to_owned();
    let lock = LockInfo::parse(
        serde_json::json!({
            "ID": Uuid::new_v4().to_string(),
            "Operation": "OperationTypeApply",
            "Who": "@host",
            "Version": "1.9.8",
            "Created": "2026-11-22T00:00:00Z",
            "Path": ""
        })
        .to_string()
        .as_bytes(),
    )?;
    backend.lock(&state_access, lock.clone()).await?;
    let serial = if opts.resources.is_empty() {
        0u64
    } else {
        let doc = state_doc(&lineage, 1, opts.resources)?;
        backend.write(&state_access, lock.id(), doc).await?;
        1u64
    };
    if !opts.leave_locked {
        backend.unlock(&state_access, lock.id()).await?;
    }
    control
        .generation_advance(&id_str, G::Creating, opts.now)
        .await?;
    control
        .generation_advance(&id_str, G::CleanupRequired, opts.now)
        .await?;
    if opts.recover_fenced {
        journal.fenced(&control_access, &process).await?;
        let rec = backend
            .recovery_records()
            .await?
            .into_iter()
            .find(|row| row.generation_id == id_str)
            .ok_or("recovery record missing")?;
        backend
            .recover_worker(&rec, FenceOutcome::Fenced, admissions, false)
            .await?;
    }
    Ok(SeededFailedCreate {
        id,
        lineage,
        serial,
    })
}
