#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(windows)]
mod common;
use shaula_core::{
    ports::{TemplateCreateRequest, TemplateRuntimePort},
    template::{BindingsDigest, GenerationIdentity, ShaulaInputEnvelope},
};

#[tokio::test]
async fn published_archive_reaches_apply_with_separate_material_provenance() {
    std::env::set_var("SHAULA_ENGINE_FENCE_EXE", env!("CARGO_BIN_EXE_shaula"));
    let tmp = tempfile::tempdir().unwrap();
    let engine = tmp.path().join("fixture.cmd");
    std::fs::write(
        &engine,
        r#"@echo off
if "%1"=="version" (echo {"terraform_version":"1.9.8"} & exit /b 0)
if "%1"=="init" (mkdir .terraform & exit /b 0)
if "%1"=="plan" (echo saved-plan>tfplan & exit /b 0)
if "%1"=="show" (type "%~dp0plan.json" & exit /b 0)
if "%1"=="apply" (copy /y "%~dp0state.json" terraform.tfstate >nul & exit /b 0)
if "%1"=="output" (type "%~dp0outputs.json" & exit /b 0)
if "%2"=="list" (if exist terraform.tfstate echo kubernetes_pod_v1.runner
exit /b 0)
if "%2"=="pull" (type terraform.tfstate & exit /b 0)
exit /b 1
"#
        .replace('\n', "\r\n"),
    )
    .unwrap();
    let resources = [
        ("kubernetes_secret_v1", "bootstrap"),
        ("kubernetes_pod_v1", "runner"),
    ];
    let plan: Vec<_> = resources.iter().map(|(kind, name)| serde_json::json!({
        "address": format!("{kind}.{name}"), "mode": "managed", "type": kind, "name": name, "change": {"actions": ["create"]}
    })).collect();
    std::fs::write(tmp.path().join("plan.json"), serde_json::json!({
        "format_version": "1.2", "terraform_version": "1.9.8", "applyable": true, "complete": true, "errored": false, "resource_changes": plan
    }).to_string()).unwrap();
    let state: Vec<_> = resources.iter().map(|(kind, name)| serde_json::json!({
        "mode": "managed", "type": kind, "name": name, "instances": [{"attributes": {"id": name}}]
    })).collect();
    std::fs::write(
        tmp.path().join("state.json"),
        serde_json::json!({"version": 4, "lineage": "original", "serial": 1, "resources": state})
            .to_string(),
    )
    .unwrap();
    std::fs::write(tmp.path().join("outputs.json"), serde_json::json!({"shaula_result": {"sensitive": true, "value": {
        "contract_version": 1, "generation_id": "g1", "bindings_digest": "commitment",
        "resources": [{"role": "bootstrap", "id": "bootstrap-id"}, {"role": "runner", "id": "runner-id"}]
    }}}).to_string()).unwrap();
    // This test exercises Terraform provenance, not an external platform launch.
    let (digest, archive) = common::artifact_variants::with_manifest(|manifest| {
        manifest.platform = "protocol-fixture".into();
        manifest.container_bootstrap_contract = None;
    })
    .unwrap();
    let published = shaula_template::ArtifactStore::new(tmp.path().join("artifacts"))
        .publish(&archive, &digest)
        .unwrap();
    let manifest = shaula_template::manifest::parse_manifest(&published.manifest_yaml).unwrap();
    let workspace = tmp.path().join("workspace");
    shaula_template::workspace::create_workspace(&workspace).unwrap();
    let runtime = shaula_template::TemplateRuntime::new(engine);
    runtime
        .prepare_create(
            &workspace,
            &published.final_path,
            &digest,
            std::time::Duration::from_secs(10),
        )
        .await
        .unwrap();
    let result = runtime
        .create(TemplateCreateRequest {
            workspace_path: workspace.clone(),
            artifact_dir: published.final_path,
            pinned_artifact_digest: digest.clone(),
            input: ShaulaInputEnvelope::new(
                GenerationIdentity {
                    fleet_key: "f1".into(),
                    scale_set_id: Some(42),
                    id: "g1".into(),
                    runner_name: "runner".into(),
                    generation_name: "generation".into(),
                },
                "private-jit".into(),
                BindingsDigest("commitment".into()),
            ),
            expected_bindings_digest: BindingsDigest("commitment".into()),
            managed_shape: manifest.managed_resource_shape,
            environment: Vec::new(),
            timeout: std::time::Duration::from_secs(10),
            apply_intent_sink: None,
            forgejo_bootstrap: None,
        })
        .await
        .unwrap();
    assert_eq!(result.provenance.artifact_digest, digest);
    assert_eq!(
        result.provenance.template_material_digest,
        shaula_template::workspace::template_files_digest(&workspace).unwrap()
    );
    assert_ne!(
        result.provenance.artifact_digest,
        result.provenance.template_material_digest
    );
    assert_eq!(result.state_lineage, "original");
    assert_eq!(result.state_serial, 1);
}
