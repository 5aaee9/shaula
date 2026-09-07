//! Real SQLite + private HTTP wire tests. The optional pinned-Terraform test
//! uses only builtin terraform_data (no Docker/Kubernetes/GitHub side effects).

// Reuse the repository's assertion-based test harness; production and the
// new backend tests themselves retain the workspace's unwrap/expect lints.
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod common;

use std::{path::PathBuf, sync::Arc, time::Duration};

use shaula_core::{
    lifecycle::GenerationState,
    ports::TemplateRuntimePort,
    registry::{ControlPlaneStore, GenerationRecord},
    state_backend::{LockInfo, StateAccess, StateBackend, StateDocument},
};
use shaula_http::state_backend::StateServer;
use shaula_store::http_state::SqliteStateBackend;
use tower::ServiceExt;
use uuid::Uuid;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

async fn backend_fixture() -> TestResult<(Arc<SqliteStateBackend>, StateAccess, axum::Router)> {
    let (app, control_plane, engine) = common::build_app_with_scan().await;
    let digest =
        common::attestation_harness::seed_profile(&app, &control_plane, "k8s-linux", true).await;
    let body = common::attest_body(
        &control_plane,
        &digest,
        &common::expected_bindings_digest("k8s-linux", 1),
        &engine,
    )
    .await;
    let (status, _) =
        common::attestation_harness::put_attestation(&app, "k8s-linux", 1, "att-1", body).await;
    assert_eq!(status, 201);
    let response = app
        .clone()
        .oneshot(common::put_with_idempotency(
            "/api/v1/fleets/linux-x64",
            "state-fixture",
            common::FLEET_BODY.to_string(),
        ))
        .await?;
    assert_eq!(response.status(), 202);
    let fleet = control_plane
        .fleet_revision_latest("linux-x64")
        .await?
        .ok_or("missing fleet")?;
    let id = Uuid::new_v4();
    let backend = Arc::new(SqliteStateBackend::new(control_plane.store().clone()));
    let (_, capability) = backend
        .insert_generation(
            GenerationRecord {
                id: id.to_string(),
                fleet_key: "linux-x64".into(),
                runner_name: format!("runner-{id}"),
                generation_name: format!("s{}", id.simple()),
                fleet_revision: 1,
                template_profile_key: "k8s-linux".into(),
                template_revision: 1,
                template_artifact_digest: digest,
                attestation_id: "att-1".into(),
                inputs_digest: fleet.inputs_digest,
                state: GenerationState::CreatePending,
                github_runner_id: None,
                workspace_path: "test-only".into(),
                created_at: 1,
                updated_at: 1,
            },
            Uuid::new_v4(),
        )
        .await?;
    Ok((
        backend,
        StateAccess {
            generation_id: id,
            capability,
        },
        app,
    ))
}

#[tokio::test]
async fn http_wire_persists_to_sqlite_and_management_oidc_stays_separate() -> TestResult {
    let (backend, access, management) = backend_fixture().await?;
    let server = StateServer::bind("127.0.0.1:0".parse()?, backend.clone()).await?;
    let config = shaula_template::http_backend::HttpBackendConfig::new(
        server.local_addr()?,
        access.generation_id,
        access.capability.clone(),
    )?;
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(server.serve(async {
        let _ = stopped.await;
    }));
    let client = reqwest::Client::builder().no_proxy().build()?;
    let request = |method: reqwest::Method| {
        client
            .request(method, config.address())
            .basic_auth("shaula-state", Some(access.capability.expose()))
    };
    assert_eq!(request(reqwest::Method::GET).send().await?.status(), 404);
    let lock = LockInfo::parse(br#"{"ID":"lock-id","Operation":"OperationTypeApply"}"#)?;
    assert_eq!(
        request(reqwest::Method::from_bytes(b"LOCK")?)
            .body(lock.to_bytes()?)
            .send()
            .await?
            .status(),
        200
    );
    let raw = br#"{"version":4,"lineage":"wire-lineage","serial":1,"outputs":{},"resources":[]}"#;
    assert_eq!(
        request(reqwest::Method::POST)
            .query(&[("ID", "wrong")])
            .body(raw.to_vec())
            .send()
            .await?
            .status(),
        423
    );
    for _ in 0..2 {
        assert_eq!(
            request(reqwest::Method::POST)
                .query(&[("ID", "lock-id")])
                .body(raw.to_vec())
                .send()
                .await?
                .status(),
            200
        );
    }
    let snapshot = backend.read(&access).await?.ok_or("state not persisted")?;
    assert_eq!(snapshot.revision, 1);
    assert_eq!(snapshot.document.bytes(), raw);
    let response = request(reqwest::Method::GET).send().await?;
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    assert_eq!(response.bytes().await?.as_ref(), raw);
    for _ in 0..2 {
        assert_eq!(
            request(reqwest::Method::from_bytes(b"UNLOCK")?)
                .body(lock.to_bytes()?)
                .send()
                .await?
                .status(),
            200
        );
    }
    assert_eq!(request(reqwest::Method::DELETE).send().await?.status(), 405);
    assert_eq!(
        client
            .get(config.address())
            .bearer_auth(common::oidc::bearer("fleet.read").trim_start_matches("Bearer "))
            .send()
            .await?
            .status(),
        401
    );
    let response = management
        .oneshot(
            axum::http::Request::builder()
                .uri("/api/v1/fleets")
                .header(
                    "authorization",
                    format!("Bearer {}", access.capability.expose()),
                )
                .body(axum::body::Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), 401);
    stop.send(()).map_err(|_| "server exited early")?;
    task.await??;
    Ok(())
}

#[tokio::test]
#[ignore = "requires SHAULA_TEST_TERRAFORM pointing to verified Terraform 1.9.8"]
async fn pinned_terraform_uses_standard_lock_query_protocol_and_keeps_state_in_database(
) -> TestResult {
    let executable = PathBuf::from(std::env::var("SHAULA_TEST_TERRAFORM")?);
    let (backend, access, _) = backend_fixture().await?;
    let server = StateServer::bind("127.0.0.1:0".parse()?, backend.clone()).await?;
    let config = shaula_template::http_backend::HttpBackendConfig::new(
        server.local_addr()?,
        access.generation_id,
        access.capability.clone(),
    )?;
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(server.serve(async {
        let _ = stopped.await;
    }));
    let temp = tempfile::tempdir()?;
    // This is a protocol fixture, NOT a conformance attestation of a Runner
    // Profile. terraform_data has no provider download or external resources.
    let mut archive = tar::Builder::new(Vec::new());
    for (name, bytes) in [
        (
            "main.tf",
            b"resource \"terraform_data\" \"runner\" { input = \"protected\" }\n".as_slice(),
        ),
        ("profile.yaml", b"protocol_fixture: true\n".as_slice()),
        (".terraform.lock.hcl", b"".as_slice()),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(u64::try_from(bytes.len())?);
        header.set_mode(0o600);
        header.set_cksum();
        archive.append_data(&mut header, name, bytes)?;
    }
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    use std::io::Write;
    encoder.write_all(&archive.into_inner()?)?;
    let archive = encoder.finish()?;
    use sha2::{Digest, Sha256};
    let digest = format!("sha256:{}", hex::encode(Sha256::digest(&archive)));
    let artifact = shaula_template::ArtifactStore::new(temp.path().join("artifacts"))
        .publish(&archive, &digest)?;
    let workspace = temp.path().join("workspace");
    let runtime =
        shaula_template::TemplateRuntime::with_http_backend(executable.clone(), config.clone());
    runtime
        .prepare_create(
            &workspace,
            &artifact.final_path,
            &digest,
            Duration::from_secs(30),
        )
        .await
        .map_err(|_| "HTTP runtime preparation failed")?;
    let metadata = std::fs::read_to_string(workspace.join(".terraform/terraform.tfstate"))?;
    assert!(
        !metadata.contains(access.capability.expose()),
        "backend password must not be cached"
    );
    std::fs::write(workspace.join("shaula.tfvars.json"), "{}")?;
    let flow =
        shaula_template::engine::TerraformFlow::open(executable, Duration::from_secs(30)).await?;
    assert_eq!(flow.version, "1.9.8");
    let env = config.environment(&[])?;
    let shape = [shaula_core::template::ManagedResourceRole {
        role: "runner".into(),
        terraform_type: "terraform_data".into(),
        exact_count: 1,
    }];
    flow.plan(&workspace, &env, false).await?;
    let plan = shaula_core::plan::parse_plan(&flow.show_plan_json(&workspace, &env).await?)?;
    shaula_core::plan::admit_create_plan(&plan, true, &shape)?;
    assert!(flow.apply_saved_plan(&workspace, &env).await?.success());
    let snapshot = backend
        .read(&access)
        .await?
        .ok_or("Terraform did not write database state")?;
    assert!(!snapshot.document.managed_empty());
    let state = flow.state_pull_snapshot(&workspace, &env).await?;
    assert_eq!(state.lineage, snapshot.document.lineage());
    flow.plan(&workspace, &env, true).await?;
    let plan = shaula_core::plan::parse_plan(&flow.show_plan_json(&workspace, &env).await?)?;
    shaula_core::plan::admit_destroy_plan(&plan, &state.managed, &shape)?;
    assert!(flow.apply_saved_plan(&workspace, &env).await?.success());
    assert!(flow.state_list(&workspace, &env).await?.is_empty());
    let final_state = backend
        .read(&access)
        .await?
        .ok_or("Destroy deleted the state row")?;
    assert!(final_state.document.managed_empty());
    assert!(final_state.revision > snapshot.revision);
    assert!(StateDocument::parse(final_state.document.bytes().to_vec())?.managed_empty());
    assert!(!workspace.join("terraform.tfstate").exists());
    assert!(!workspace.join("errored.tfstate").exists());
    let lock = LockInfo::parse(br#"{"ID":"after-destroy"}"#)?;
    backend.lock(&access, lock.clone()).await?;
    backend.unlock(&access, lock.id()).await?;
    stop.send(()).map_err(|_| "server exited early")?;
    task.await??;
    Ok(())
}
