//! End-to-end verification for ARD-0044:
//! - Static HCL read-only validation of `reconcile.discover_module` and contract bounds
//! - Phase 1 structured `AnchorOutcome` (`LockHeld`, `ShapeIncomplete`, `Anchored`)
//! - Phase 2 failed-Create reconciliation via `FleetSupervisor::tick` with real
//!   SQLite + `SqliteStateBackend` (empty-state settle + prove empty, partial-state
//!   reconcile + destroy, unimportable resource fail-closed to Quarantine, transient
//!   discover failure backoff + retry, and opt-out template unchanged Quarantine).
//!
//! Writes verifiable evidence to `$CARGO_TARGET_TMPDIR/ard-0044-reconcile/report.json`.

#[path = "support/reconcile_fixture.rs"]
mod fixture;
#[path = "support/reconcile_guards.rs"]
mod guards;

use std::sync::{atomic::Ordering, Arc};

use sea_orm::ConnectionTrait;
use shaula_core::lifecycle::GenerationState as G;
use shaula_core::ports::{ReconcileClassification, TemplateOutcomeError};
use shaula_core::registry::LifecycleStore;
use shaula_core::state_backend::{AnchorOutcome, AnchorSkipReason};
use shaula_daemon::supervisor::{
    FleetSupervisor, FleetSupervisorConfig, FleetSupervisorDeps, LifecycleLimits,
};
use shaula_store::http_state::{SqliteStateBackend, WorkerAdmissions};
use shaula_store::registry_impl::SqliteControlPlane;
use shaula_store::Store;

use fixture::{make_reconcile_artifact, seed_failed_create, ScriptedRuntime, SeedOpts, TestResult};
use guards::{seed_registry_rows, verify_static_reconcile_guards, StubGitHub};

#[tokio::test]
async fn ard_0044_failed_create_reconcile_end_to_end() -> TestResult {
    verify_static_reconcile_guards()?;

    let tmp = tempfile::tempdir()?;
    let artifact_root = tmp.path().join("artifacts");
    let db_path = tmp.path().join("reconcile.db");
    let store = Store::open(&db_path).await?;
    store.migrate().await?;
    let backend = SqliteStateBackend::new(store.clone());
    backend.activate().await?;
    let admissions = Arc::new(WorkerAdmissions::new(16, 4)?);
    let control = Arc::new(
        SqliteControlPlane::new(store.clone(), artifact_root.clone())
            .with_worker_admissions(admissions.clone()),
    );
    let (reconcile_digest, _) = make_reconcile_artifact(&artifact_root, 65, true)?;
    let (plain_digest, _) = make_reconcile_artifact(&artifact_root, 0, false)?;

    let db = sea_orm::Database::connect(format!("sqlite://{}?mode=rw", db_path.display())).await?;
    seed_registry_rows(&db, &artifact_root, &reconcile_digest, &plain_digest).await?;

    // Phase 1: structured AnchorOutcome (LockHeld -> Anchored after fencing).
    let locked_whole = seed_failed_create(
        &control,
        &backend,
        &admissions,
        SeedOpts {
            profile_key: "pve-reconcile",
            digest: &reconcile_digest,
            resources: &[
                ("proxmox_nocloud_iso", "bootstrap"),
                ("proxmox_qemu_vm", "runner"),
            ],
            leave_locked: true,
            recover_fenced: false,
            now: 10_000,
        },
    )
    .await?;
    assert_eq!(
        control
            .generation_anchor_failed_create(&locked_whole.id.to_string())
            .await?,
        AnchorOutcome::Skipped(AnchorSkipReason::LockHeld)
    );
    db.execute(sea_orm::Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "UPDATE lifecycle_workers SET phase = 'fenced' WHERE generation_id = ?",
        vec![locked_whole.id.to_string().into()],
    ))
    .await?;
    assert_eq!(
        control
            .generation_anchor_failed_create(&locked_whole.id.to_string())
            .await?,
        AnchorOutcome::Anchored
    );
    assert_eq!(
        control
            .generation_state_identity(&locked_whole.id.to_string())
            .await?,
        Some((locked_whole.lineage.clone(), locked_whole.serial))
    );
    db.execute(sea_orm::Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "UPDATE runner_generations SET state = 'Destroyed' WHERE id = ?",
        vec![locked_whole.id.to_string().into()],
    ))
    .await?;

    // Scenario E (opt-out template without reconcile): partial state still quarantines.
    let plain_partial = seed_failed_create(
        &control,
        &backend,
        &admissions,
        SeedOpts {
            profile_key: "pve-plain",
            digest: &plain_digest,
            resources: &[("proxmox_nocloud_iso", "bootstrap")],
            leave_locked: true,
            recover_fenced: true,
            now: 10_000,
        },
    )
    .await?;
    assert_eq!(
        control
            .generation_get(&plain_partial.id.to_string())
            .await?
            .ok_or("plain gen")?
            .state,
        G::Quarantined
    );

    // Scenario A (44/49 empty state + settle_delay_secs=65 gate -> Destroyed).
    let empty_gen = seed_failed_create(
        &control,
        &backend,
        &admissions,
        SeedOpts {
            profile_key: "pve-reconcile",
            digest: &reconcile_digest,
            resources: &[],
            leave_locked: true,
            recover_fenced: true,
            now: 10_000,
        },
    )
    .await?;
    assert_eq!(
        control
            .generation_get(&empty_gen.id.to_string())
            .await?
            .ok_or("empty gen")?
            .state,
        G::CleanupRequired
    );
    assert_eq!(
        control
            .generation_state_identity(&empty_gen.id.to_string())
            .await?,
        None
    );
    assert_eq!(
        control
            .generation_anchor_failed_create(&empty_gen.id.to_string())
            .await?,
        AnchorOutcome::Skipped(AnchorSkipReason::ShapeIncomplete)
    );

    let github = Arc::new(StubGitHub);
    let runtime = Arc::new(ScriptedRuntime::new(vec![Ok(
        ReconcileClassification::Empty {
            state_lineage: empty_gen.lineage.clone(),
            state_serial: empty_gen.serial,
        },
    )]));
    let supervisor = FleetSupervisor::new(
        FleetSupervisorDeps {
            limits: LifecycleLimits {
                create: Arc::new(tokio::sync::Semaphore::new(2)),
                destroy: Arc::new(tokio::sync::Semaphore::new(2)),
                template_create: Default::default(),
            },
            store: control.clone(),
            handoff: control.clone(),
            github,
            runtime: runtime.clone(),
        },
        FleetSupervisorConfig {
            fleet_key: "f1".into(),
            capacity: shaula_core::capacity::CapacityPolicy {
                min_runners: 0,
                max_runners: 10,
            },
            work_root: tmp.path().join("work"),
            artifact_root,
            operation_timeout: std::time::Duration::from_secs(5),
            apply_intent_sink: Arc::new(shaula_daemon::apply_intent::LedgerApplyIntentSink {
                store: control.clone(),
                gates: Arc::new(shaula_daemon::effect_gate::FleetEffectGates::default()),
            }),
            labels: vec![shaula_core::github::Label::system("shaula-x64")],
            auth_profile_key: "prod-app".into(),
            auth_revision: 1,
        },
        shaula_core::github::ScaleSetIdentity {
            target: shaula_core::github::GitHubTarget::organization("example-org")?,
            runner_group: "Default".into(),
            scale_set_name: "shaula-x64".into(),
        },
    );

    supervisor.tick(70_000).await?;
    assert_eq!(runtime.reconcile_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        control
            .generation_get(&empty_gen.id.to_string())
            .await?
            .ok_or("empty gen")?
            .state,
        G::Destroying
    );
    supervisor.tick(135_000).await?;
    assert_eq!(runtime.reconcile_calls.load(Ordering::SeqCst), 1);
    assert_eq!(runtime.destroy_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        control
            .generation_get(&empty_gen.id.to_string())
            .await?
            .ok_or("empty gen")?
            .state,
        G::Destroyed
    );

    // Scenario B (5/49 partial 1/2 state -> reconcile Anchored -> normal Destroy -> Destroyed).
    let partial_gen = seed_failed_create(
        &control,
        &backend,
        &admissions,
        SeedOpts {
            profile_key: "pve-reconcile",
            digest: &reconcile_digest,
            resources: &[("proxmox_nocloud_iso", "bootstrap")],
            leave_locked: true,
            recover_fenced: true,
            now: 200_000,
        },
    )
    .await?;
    runtime
        .reconcile_responses
        .lock()
        .map_err(|_| "lock")?
        .push_back(Ok(ReconcileClassification::Anchored {
            state_lineage: partial_gen.lineage.clone(),
            state_serial: partial_gen.serial,
            adopted_count: 0,
        }));
    *runtime.destroy_db.lock().map_err(|_| "lock")? = Some((
        db.clone(),
        partial_gen.id.to_string(),
        partial_gen.lineage.clone(),
    ));
    supervisor.tick(260_000).await?;
    supervisor.tick(326_000).await?;
    assert_eq!(runtime.destroy_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        control
            .generation_get(&partial_gen.id.to_string())
            .await?
            .ok_or("partial gen")?
            .state,
        G::Destroyed
    );
    *runtime.destroy_db.lock().map_err(|_| "lock")? = None;

    // Scenario C (Unimportable untracked ISO -> fail closed to Quarantined).
    let unimportable_gen = seed_failed_create(
        &control,
        &backend,
        &admissions,
        SeedOpts {
            profile_key: "pve-reconcile",
            digest: &reconcile_digest,
            resources: &[],
            leave_locked: true,
            recover_fenced: true,
            now: 400_000,
        },
    )
    .await?;
    runtime
        .reconcile_responses
        .lock()
        .map_err(|_| "lock")?
        .push_back(Ok(ReconcileClassification::Unimportable {
            address: "proxmox_nocloud_iso.bootstrap".into(),
        }));
    supervisor.tick(460_000).await?;
    supervisor.tick(526_000).await?;
    assert_eq!(
        control
            .generation_get(&unimportable_gen.id.to_string())
            .await?
            .ok_or("unimportable gen")?
            .state,
        G::Quarantined
    );

    // Scenario D (Transient provider 5xx during discover -> backoff retry -> Destroyed).
    let transient_gen = seed_failed_create(
        &control,
        &backend,
        &admissions,
        SeedOpts {
            profile_key: "pve-reconcile",
            digest: &reconcile_digest,
            resources: &[],
            leave_locked: true,
            recover_fenced: true,
            now: 600_000,
        },
    )
    .await?;
    {
        let mut q = runtime.reconcile_responses.lock().map_err(|_| "lock")?;
        q.push_back(Err(TemplateOutcomeError::ExecutionFailed {
            phase: "reconcile.discover".into(),
        }));
        q.push_back(Ok(ReconcileClassification::Empty {
            state_lineage: transient_gen.lineage.clone(),
            state_serial: transient_gen.serial,
        }));
    }
    supervisor.tick(660_000).await?;
    let calls_before = runtime.reconcile_calls.load(Ordering::SeqCst);
    supervisor.tick(726_000).await?;
    assert_eq!(
        runtime.reconcile_calls.load(Ordering::SeqCst),
        calls_before + 1
    );
    assert_eq!(
        control
            .generation_get(&transient_gen.id.to_string())
            .await?
            .ok_or("transient gen")?
            .state,
        G::Destroying
    );
    supervisor.tick(727_000).await?;
    assert_eq!(
        runtime.reconcile_calls.load(Ordering::SeqCst),
        calls_before + 1
    );
    supervisor.tick(800_000).await?;
    assert_eq!(
        runtime.reconcile_calls.load(Ordering::SeqCst),
        calls_before + 2
    );
    assert_eq!(
        control
            .generation_get(&transient_gen.id.to_string())
            .await?
            .ok_or("transient gen")?
            .state,
        G::Destroyed
    );

    let report_dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("ard-0044-reconcile");
    std::fs::create_dir_all(&report_dir)?;
    std::fs::write(
        report_dir.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "ard": "0044",
            "phase1_lock_held_then_anchored": true,
            "scenario_a_empty_settle_and_destroyed": empty_gen.id.to_string(),
            "scenario_b_partial_anchored_and_destroyed": partial_gen.id.to_string(),
            "scenario_c_unimportable_quarantined": unimportable_gen.id.to_string(),
            "scenario_d_transient_retry_and_destroyed": transient_gen.id.to_string(),
            "scenario_e_opt_out_unchanged_quarantined": plain_partial.id.to_string(),
        }))?,
    )?;
    Ok(())
}
