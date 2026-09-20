//! Real Shaula re-exec/fence plus fixture Terraform, Docker and SSH executables.
//! Verifies credential plumbing across Create/bootstrap/Destroy, not remote
//! Engine conformance or the SSH protocol itself.
#![cfg(unix)]

#[path = "docker_ssh/fixtures.rs"]
mod fixtures;

use fixtures::{TestResult, GENERATION, HOST};
use serde_json::json;
use shaula_core::{
    ports::{
        ApplyClaim, ApplyIntentSink, DestroyClassification, OriginalStateIdentity, PlanProvenance,
        TemplateCreateRequest, TemplateDestroyRequest, TemplateOutcomeError, TemplateRuntimePort,
    },
    template::{BindingsDigest, GenerationIdentity, ShaulaInputEnvelope},
};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

#[derive(Default)]
struct Admission(AtomicUsize);

#[async_trait::async_trait]
impl ApplyIntentSink for Admission {
    async fn persist_apply_starting(&self, _: &PlanProvenance) -> Result<ApplyClaim, String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(()))
    }
    async fn authorize_bootstrap(&self, _: &PlanProvenance) -> Result<ApplyClaim, String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(()))
    }
}

struct Environment(Vec<(&'static str, Option<std::ffi::OsString>)>);

impl Environment {
    fn new(root: &Path) -> TestResult<Self> {
        let original = std::env::var_os("PATH");
        let mut paths = vec![root.to_path_buf()];
        paths.extend(std::env::split_paths(
            original.as_deref().ok_or("PATH missing")?,
        ));
        let guard = Self(vec![
            ("PATH", original),
            (
                "SHAULA_ENGINE_FENCE_EXE",
                std::env::var_os("SHAULA_ENGINE_FENCE_EXE"),
            ),
        ]);
        std::env::set_var("PATH", std::env::join_paths(paths)?);
        std::env::set_var("SHAULA_ENGINE_FENCE_EXE", env!("CARGO_BIN_EXE_shaula"));
        Ok(guard)
    }
}

impl Drop for Environment {
    fn drop(&mut self) {
        for (key, value) in &self.0 {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

#[tokio::test]
async fn docker_ssh_authentication_survives_create_bootstrap_and_reconstructed_destroy(
) -> TestResult {
    // One test owns process-wide fixture executable resolution; no parallel
    // tests in this binary mutate the daemon PATH/supervisor test seam.
    for authentication in [
        json!({"ssh_password":"password-secret"}),
        json!({"ssh_private_key":"private-key"}),
        json!({"ssh_private_key":"private-key", "ssh_private_key_passphrase":"passphrase-secret"}),
        json!({"ssh_password":"wrong-secret"}),
    ] {
        let temp = tempfile::Builder::new()
            .prefix("shaula ssh test ")
            .tempdir()?;
        let root = temp.path();
        let published = fixtures::artifact(root)?;
        let manifest = shaula_template::manifest::parse_manifest(&published.manifest_yaml)?;
        fixtures::responses(
            root,
            manifest
                .runner_image_digests
                .first()
                .ok_or("image missing")?,
        )?;
        let engine = fixtures::script(root, "terraform", fixtures::ENGINE)?;
        fixtures::script(root, "ssh", fixtures::SSH)?;
        fixtures::script(root, "docker", fixtures::DOCKER)?;
        let _environment = Environment::new(root)?;
        let workspace = root.join("workspace");
        shaula_template::workspace::create_workspace(&workspace)?;
        let mut input = ShaulaInputEnvelope::new(
            GenerationIdentity {
                fleet_key: "fleet".into(),
                scale_set_id: Some(1),
                id: GENERATION.into(),
                runner_name: "runner".into(),
                generation_name: "generation".into(),
            },
            "frozen-jit".into(),
            BindingsDigest("commitment".into()),
        );
        input.bindings = serde_json::from_value(authentication.clone())?;
        input.bindings.insert("docker_host".into(), json!(HOST));
        input.bindings.insert(
            "ssh_known_hosts".into(),
            json!("[host]:2222 ssh-ed25519 public-key\n"),
        );
        let sink = Arc::new(Admission::default());
        let runtime = shaula_template::TemplateRuntime::new(engine);
        let created = runtime
            .create(TemplateCreateRequest {
                workspace_path: workspace.clone(),
                artifact_dir: published.final_path.clone(),
                pinned_artifact_digest: published.digest.clone(),
                expected_bindings_digest: input.bindings_digest.clone(),
                managed_shape: manifest.managed_resource_shape.clone(),
                input,
                environment: Vec::new(),
                timeout: Duration::from_secs(60),
                apply_intent_sink: Some(sink.clone()),
                forgejo_bootstrap: None,
            })
            .await;
        if authentication["ssh_password"] == "wrong-secret" {
            assert_eq!(
                created.err(),
                Some(TemplateOutcomeError::PlanFailed {
                    phase: "create.plan".into()
                })
            );
            assert_eq!(sink.0.load(Ordering::SeqCst), 0);
            continue;
        }
        let created = created.map_err(|error| format!("create: {error:?}"))?;
        assert_eq!(sink.0.load(Ordering::SeqCst), 2);
        let original_input = std::fs::read(workspace.join("shaula.tfvars.json"))?;
        let create_calls = std::fs::read_to_string(root.join("calls"))?;
        assert!(create_calls.contains("bootstrap-container-start|"));
        assert!(create_calls.contains("bootstrap-cp---|"));
        let create_paths = assert_helpers_removed(&create_calls)?;
        assert_eq!(
            create_paths.len(),
            1,
            "Terraform and bootstrap must share a helper"
        );

        // A new Runtime has no transient authentication state from Create.
        let runtime = shaula_template::TemplateRuntime::new(root.join("terraform"));
        let result = runtime
            .destroy(TemplateDestroyRequest {
                workspace_path: workspace.clone(),
                artifact_dir: published.final_path,
                pinned_artifact_digest: published.digest,
                expected_bindings_digest: BindingsDigest("commitment".into()),
                generation_id: GENERATION.into(),
                managed_shape: manifest.managed_resource_shape,
                environment: Vec::new(),
                timeout: Duration::from_secs(10),
                original_state: OriginalStateIdentity {
                    lineage: created.state_lineage,
                    serial: created.state_serial,
                    allow_serial_advance: false,
                },
                original_provenance: created.provenance,
                apply_intent_sink: Some(sink.clone()),
            })
            .await
            .map_err(|error| format!("destroy: {error:?}"))?;
        assert_eq!(result, DestroyClassification::Applied);
        assert_eq!(sink.0.load(Ordering::SeqCst), 3);
        assert_eq!(
            std::fs::read(workspace.join("shaula.tfvars.json"))?,
            original_input
        );
        let calls = std::fs::read_to_string(root.join("calls"))?;
        assert_eq!(
            assert_helpers_removed(&calls)?.len(),
            2,
            "Destroy must reconstruct disposable files"
        );
        assert!(!calls.contains("secret") && !calls.contains("private-key"));
    }
    Ok(())
}

fn assert_helpers_removed(calls: &str) -> TestResult<std::collections::BTreeSet<&str>> {
    let mut paths = std::collections::BTreeSet::new();
    for call in calls.lines() {
        let (_, path) = call.split_once('|').ok_or("missing helper path")?;
        assert!(
            !Path::new(path).exists(),
            "temporary authentication must be removed"
        );
        paths.insert(path);
    }
    Ok(paths)
}
