//! New container admission is strict while retained cleanup stays executable.
use crate::common;

use axum::http::StatusCode;
use sha2::{Digest, Sha256};
use shaula_core::{
    ports::{
        DestroyClassification, OriginalStateIdentity, PlanProvenance, StateLineage,
        TemplateCreateRequest, TemplateDestroyRequest, TemplateOutcomeError, TemplateRuntimePort,
    },
    registry::ControlPlaneStore,
    template::{BindingsDigest, GenerationIdentity, ShaulaInputEnvelope},
};
use std::{path::Path, time::Duration};
use tower::ServiceExt;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn legacy_archive() -> Result<(String, Vec<u8>), Box<dyn std::error::Error>> {
    common::artifact_variants::with_manifest(|manifest| {
        manifest.container_bootstrap_contract = None;
        manifest.runner_image_digests = vec![format!(
            "registry.test/custom:old@sha256:{}",
            "a".repeat(64)
        )];
    })
}

fn input() -> ShaulaInputEnvelope {
    ShaulaInputEnvelope::new(
        GenerationIdentity {
            fleet_key: "legacy-fleet".into(),
            scale_set_id: 1,
            id: "legacy-generation".into(),
            runner_name: "legacy-runner".into(),
            generation_name: "legacy-resource".into(),
        },
        "protected-jit".into(),
        BindingsDigest("commitment".into()),
    )
}

#[tokio::test]
async fn retained_artifact_cannot_be_published_as_a_new_legacy_container_revision() -> TestResult {
    let (app, store, _) = common::build_app_with_scan().await;
    let (digest, archive) = legacy_archive()?;
    let mut upload =
        common::authorized("PUT", &format!("/api/v1/template-artifacts/{digest}"), None);
    *upload.body_mut() = axum::body::Body::from(archive);
    // This test's low-level cache publisher models an already retained archive.
    assert_eq!(
        app.clone().oneshot(upload).await?.status(),
        StatusCode::CREATED
    );
    let response = app
        .oneshot(common::authorized(
            "PUT",
            "/api/v1/template-profiles/legacy",
            Some(common::TEMPLATE_PUT_BODY.replace("PLACEHOLDER", &digest)),
        ))
        .await?;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(store.template_profile_get("legacy").await?.is_none());
    Ok(())
}

#[tokio::test]
async fn legacy_container_create_and_prepare_are_rejected_before_engine_spawn() -> TestResult {
    let temp = tempfile::tempdir()?;
    let (digest, archive) = legacy_archive()?;
    let published = shaula_template::ArtifactStore::new(temp.path().join("artifacts"))
        .publish(&archive, &digest)?;
    let manifest = shaula_template::manifest::parse_manifest(&published.manifest_yaml)?;
    let workspace = temp.path().join("workspace");
    let runtime = shaula_template::TemplateRuntime::new(temp.path().join("must-not-execute"));
    let rejected = TemplateOutcomeError::StateUnavailable {
        phase: "input.contract".into(),
    };
    assert_eq!(
        runtime
            .prepare_create(
                &workspace,
                &published.final_path,
                &digest,
                Duration::from_secs(1)
            )
            .await,
        Err(rejected.clone())
    );
    let result = runtime
        .create(TemplateCreateRequest {
            workspace_path: workspace.clone(),
            artifact_dir: published.final_path,
            pinned_artifact_digest: digest,
            input: input(),
            expected_bindings_digest: BindingsDigest("commitment".into()),
            managed_shape: manifest.managed_resource_shape,
            environment: Vec::new(),
            timeout: Duration::from_secs(1),
            apply_intent_sink: None,
        })
        .await;
    assert!(matches!(result, Err(error) if error == rejected));
    assert!(!workspace.exists());
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

fn cleanup_engine(directory: &Path) -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    #[cfg(windows)]
    let (name, body) = ("engine.cmd", "@echo off\r\nif \"%1\"==\"version\" (echo {\"terraform_version\":\"1.9.8\"} & exit /b 0)\r\nif \"%1\"==\"state\" (type terraform.tfstate & exit /b 0)\r\nexit /b 1\r\n");
    #[cfg(not(windows))]
    let (name, body) = ("engine.sh", "#!/bin/sh\nif [ \"$1\" = version ]; then printf '%s\\n' '{\"terraform_version\":\"1.9.8\"}'; exit 0; fi\nif [ \"$1\" = state ]; then /bin/cat terraform.tfstate; exit 0; fi\nexit 1\n");
    let path = directory.join(name);
    std::fs::write(&path, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(path)
}

#[tokio::test]
async fn legacy_container_destroy_keeps_original_manifest_and_provenance_contract() -> TestResult {
    std::env::set_var("SHAULA_ENGINE_FENCE_EXE", env!("CARGO_BIN_EXE_shaula"));
    let temp = tempfile::tempdir()?;
    let (artifact_digest, archive) = legacy_archive()?;
    let published = shaula_template::ArtifactStore::new(temp.path().join("artifacts"))
        .publish(&archive, &artifact_digest)?;
    let manifest = shaula_template::manifest::parse_manifest(&published.manifest_yaml)?;
    assert!(manifest.validate_new_container_profile().is_err());
    let workspace = temp.path().join("workspace");
    std::fs::create_dir(&workspace)?;
    shaula_template::artifact::extract_tar_gz(&archive, &workspace, 1024 * 1024)?;
    let protected_input = serde_json::to_vec(&serde_json::json!({"shaula":input()}))?;
    std::fs::write(workspace.join("shaula.tfvars.json"), &protected_input)?;
    std::fs::write(
        workspace.join("terraform.tfstate"),
        br#"{"version":4,"lineage":"legacy-lineage","serial":3,"resources":[]}"#,
    )?;
    let engine = cleanup_engine(temp.path())?;
    let material = shaula_template::workspace::template_files_digest(&workspace)?;
    let provenance = PlanProvenance {
        intent: shaula_core::plan::PlanIntent::Create,
        saved_plan_digest: "original-plan".into(),
        engine_kind: "terraform".into(),
        engine_version: "1.9.8".into(),
        engine_binary_digest: digest(&std::fs::read(&engine)?),
        artifact_digest: artifact_digest.clone(),
        template_material_digest: material,
        protected_input_digest: digest(&protected_input),
        state_lineage: StateLineage::Empty,
        generation_id: "legacy-generation".into(),
        attempt_id: "original-create".into(),
    };
    let outcome = shaula_template::TemplateRuntime::new(engine)
        .destroy(TemplateDestroyRequest {
            pinned_artifact_digest: artifact_digest,
            workspace_path: workspace,
            artifact_dir: published.final_path,
            expected_bindings_digest: BindingsDigest("commitment".into()),
            generation_id: "legacy-generation".into(),
            managed_shape: manifest.managed_resource_shape,
            environment: Vec::new(),
            timeout: Duration::from_secs(10),
            original_provenance: provenance,
            original_state: OriginalStateIdentity {
                lineage: "legacy-lineage".into(),
                serial: 3,
                allow_serial_advance: false,
            },
            apply_intent_sink: None,
        })
        .await
        .map_err(|error| format!("legacy cleanup rejected: {error:?}"))?;
    assert_eq!(outcome, DestroyClassification::AlreadyEmpty);
    Ok(())
}
