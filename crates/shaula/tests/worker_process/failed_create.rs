//! Spec 0004 §5.7 with the real worker and Terraform: a failed Create whose
//! state records the whole managed shape is anchored and destroyed, while a
//! partial shape stays unanchored. Each case writes its evidence to
//! `$CARGO_TARGET_TMPDIR/failed-create/<case>.json`.
use super::artifact::Template;
use super::fixture::{Fixture, TestResult};
use shaula_core::{lifecycle::GenerationState as G, registry::LifecycleStore, worker::*};
use std::time::Duration;

async fn failed(template: Template) -> TestResult<(Fixture, String)> {
    let fixture = Fixture::with_template(template).await?;
    let error = fixture
        .apply_create()
        .await?
        .err()
        .ok_or("the Create apply must fail")?;
    Ok((fixture, error))
}

fn evidence(case: &str, report: serde_json::Value) -> TestResult {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("failed-create");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        dir.join(format!("{case}.json")),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}

/// Destroys from the anchored identity and the Create's own provenance,
/// exactly what a later supervisor Destroy reads from the ledger.
async fn destroy_anchored(fixture: &Fixture) -> TestResult<(String, u64)> {
    let id = fixture.id.to_string();
    let identity = fixture
        .control
        .generation_state_identity(&id)
        .await?
        .ok_or("anchored identity missing")?;
    let provenance = fixture
        .control
        .operation_original_provenance(&id)
        .await?
        .ok_or("Create provenance missing")?;
    fixture.destroy_from(provenance, identity.clone()).await?;
    assert!(fixture.state().await?.managed_empty());
    assert_eq!(fixture.create_count().await?, 1, "Destroy never re-creates");
    Ok(identity)
}

#[tokio::test]
#[ignore = "requires SHAULA_TEST_CGROUP and SHAULA_TEST_TERRAFORM"]
async fn failed_create_recording_the_whole_shape_is_anchored_and_destroyed() -> TestResult {
    let (fixture, error) = failed(Template::FailsTainted).await?;
    let id = fixture.id.to_string();
    let state = fixture.state().await?;
    assert_eq!(state.managed_counts().get("terraform_data"), Some(&1));
    assert_eq!(fixture.control.generation_state_identity(&id).await?, None);
    assert!(fixture.control.generation_anchor_failed_create(&id).await?);
    assert!(
        fixture.control.generation_anchor_failed_create(&id).await?,
        "a repeated anchor verifies the same identity"
    );
    let identity = destroy_anchored(&fixture).await?;
    assert_eq!(identity, (state.lineage().into(), state.serial() as u64));
    fixture
        .control
        .generation_advance(&id, G::Destroyed, 10)
        .await?;
    evidence(
        "whole-shape",
        serde_json::json!({
            "generation": id, "create_error": error,
            "failed_state": {"lineage": identity.0, "serial": identity.1, "managed": state.managed_counts()},
            "anchored": true, "destroyed": true,
        }),
    )?;
    fixture.close().await
}

#[tokio::test]
#[ignore = "requires SHAULA_TEST_CGROUP and SHAULA_TEST_TERRAFORM"]
async fn failed_create_recording_part_of_the_shape_stays_unanchored() -> TestResult {
    let (fixture, error) = failed(Template::FailsPartial).await?;
    let id = fixture.id.to_string();
    let state = fixture.state().await?;
    assert_eq!(
        state.managed_counts().get("terraform_data"),
        Some(&1),
        "the refused second runner never reached the state: {error}"
    );
    assert!(!fixture.control.generation_anchor_failed_create(&id).await?);
    assert_eq!(fixture.control.generation_state_identity(&id).await?, None);
    assert!(!state.managed_empty(), "nothing is destroyed on a guess");
    evidence(
        "partial-shape",
        serde_json::json!({
            "generation": id, "create_error": error,
            "failed_state": {"lineage": state.lineage(), "serial": state.serial(), "managed": state.managed_counts()},
            "expected_shape": {"terraform_data": 2}, "anchored": false,
        }),
    )?;
    fixture.close().await
}

#[tokio::test]
#[ignore = "requires SHAULA_TEST_CGROUP and SHAULA_TEST_TERRAFORM"]
async fn restart_recovery_anchors_a_fenced_failed_create() -> TestResult {
    // The daemon stops before anchoring: shutdown fences the live worker, and
    // restart recovery is the first to see the failed state. Its cleanup-only
    // replacement destroys as `fenced_worker_replacement_restores_missing_
    // workspace_without_create` shows, under a fresh `Workers`.
    let (fixture, error) = failed(Template::FailsTainted).await?;
    let id = fixture.id.to_string();
    fixture.workers.shutdown(Duration::from_secs(1)).await?;
    let record = fixture
        .backend
        .recovery_records()
        .await?
        .pop()
        .ok_or("worker absent")?;
    assert_eq!(record.phase, "fenced");
    assert_eq!(fixture.control.generation_state_identity(&id).await?, None);
    fixture
        .backend
        .recover_worker(&record, FenceOutcome::Fenced, &fixture.admissions, false)
        .await?;
    let generation = fixture
        .control
        .generation_get(&id)
        .await?
        .ok_or("generation absent")?;
    assert_eq!(generation.state, G::CleanupRequired, "not Quarantined");
    let state = fixture.state().await?;
    let anchored = fixture
        .control
        .generation_state_identity(&id)
        .await?
        .ok_or("recovery did not anchor")?;
    assert_eq!(anchored, (state.lineage().into(), state.serial() as u64));
    let replacement = fixture
        .backend
        .recovery_records()
        .await?
        .pop()
        .ok_or("replacement absent")?;
    assert_eq!(replacement.phase, "admitted");
    assert_eq!(replacement.worker_epoch, record.worker_epoch + 1);
    evidence(
        "fenced-recovery",
        serde_json::json!({
            "generation": id, "create_error": error,
            "recovered_state": format!("{:?}", generation.state),
            "anchored": {"lineage": anchored.0, "serial": anchored.1},
            "replacement": {"phase": replacement.phase, "worker_epoch": replacement.worker_epoch},
        }),
    )?;
    fixture.stop_server().await
}
