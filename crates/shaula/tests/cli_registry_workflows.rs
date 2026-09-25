//! PAT CLI parity over the same registry/attestation fixture as HTTP tests.
// Existing shared fixture helpers intentionally panic on invalid test setup.
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod common;
use common::*;
use serde_json::{json, Value};
use shaula_client::{Client, Secret};
use shaula_core::{
    access_tokens::{IssueToken, TokenPolicy, TokenService},
    registry::{Actor, AuthenticationContext, Scope},
};
use std::sync::Arc;
type TestResult = Result<(), Box<dyn std::error::Error>>;

struct Fixture {
    client: Client,
    secret: Secret,
    task: tokio::task::JoinHandle<()>,
    digest: String,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let (app, store, engine, root) = build_app_with_artifact_root().await;
        let digest =
            common::attestation_harness::seed_profile(&app, &store, "k8s-linux", true).await;
        let body = attest_body(
            &store,
            &digest,
            &expected_bindings_digest("k8s-linux", 1),
            &engine,
        )
        .await;
        assert_eq!(
            common::attestation_harness::put_attestation(&app, "k8s-linux", 1, "att", body)
                .await
                .0,
            axum::http::StatusCode::CREATED
        );
        let identity = oidc::oidc().await;
        let (realm, mut grants) = identity.token_context();
        let principal = serde_json::to_string(&("oidc-v1", &oidc::provider().issuer, "ops"))?;
        grants.insert(principal.clone(), Scope::ALL.to_vec());
        let tokens = shaula_daemon::access_tokens::PersonalTokens::new(
            Arc::new(store.store().clone()),
            fixed_clock(),
            TokenPolicy::default(),
            realm,
            grants,
        )?;
        let actor = Actor {
            name: principal,
            scopes: Scope::ALL.to_vec(),
            authentication: AuthenticationContext::OidcAccessToken,
        };
        let issued = tokens
            .issue(
                &actor,
                true,
                IssueToken {
                    name: "registry fixture".into(),
                    scopes: Scope::ALL
                        .iter()
                        .filter(|s| s.delegable())
                        .map(|s| s.as_str().into())
                        .collect(),
                    expires_in_seconds: Some(3600),
                },
                "registry",
            )
            .await?;
        let secret = Secret::new(issued.secret.ok_or("secret")?.expose().into());
        let control = Arc::new(shaula_daemon::service::ControlPlane::new(
            store.clone(),
            fixed_clock(),
            b"test-bindings-key".to_vec(),
            100,
            engine,
        ));
        control.set_ready(true);
        let app = shaula_http::router::build_router(shaula_http::router::AppState {
            access_tokens: Some(Arc::new(tokens)),
            auth_installation_link: None,
            fleets: control.clone(),
            profiles: control.clone(),
            pools: control.clone(),
            health: control,
            oidc: identity,
            body_limit: 64 * 1024 * 1024,
            request_body_limit: 64 * 1024 * 1024,
            jobs: Some(Arc::new(store.store().clone())),
            logs: None,
            artifact_publisher: Arc::new(TestPublisher { root }),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self {
            client: Client::loopback(&origin, secret.clone())?,
            secret,
            task,
            digest,
        })
    }
    async fn cli(&self, args: Vec<String>) -> Result<(i32, Value), Box<dyn std::error::Error>> {
        let origin = self.client.origin().to_owned();
        let secret = self.secret.clone();
        let output = tokio::task::spawn_blocking(move || {
            std::process::Command::new(env!("CARGO_BIN_EXE_shaula"))
                .args([
                    "--server",
                    &origin,
                    "--allow-loopback-http",
                    "--output",
                    "json",
                ])
                .args(args)
                .env("SHAULA_ACCESS_TOKEN", secret.expose())
                .env_remove("SHAULA_CONTEXT")
                .output()
        })
        .await??;
        Ok((
            output.status.code().ok_or("code")?,
            serde_json::from_slice(&output.stdout)?,
        ))
    }
}
fn args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).into()).collect()
}

#[tokio::test]
async fn fleet_and_pool_create_update_conflict_retire_and_typed_views() -> TestResult {
    let fixture = Fixture::new().await?;
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("fleet.json");
    std::fs::write(&path, FLEET_BODY)?;
    let path = path.to_string_lossy();
    let (code, created) = fixture
        .cli(args(&["fleets", "create", "cli-fleet", "--file", &path]))
        .await?;
    assert_eq!(code, 0, "{created}");
    assert_eq!(created["receipt"]["outcome"], "accepted");
    let resource = fixture.client.fleets().get_typed("cli-fleet").await?;
    let version = resource.version.ok_or("version")?;
    assert_eq!(
        fixture
            .client
            .fleets()
            .status_typed("cli-fleet")
            .await?
            .data
            .fleet_key,
        "cli-fleet"
    );
    let mut body: Value = serde_json::from_str(FLEET_BODY)?;
    body["capacity"]["max_runners"] = json!(6);
    std::fs::write(path.as_ref(), serde_json::to_vec(&body)?)?;
    let update = args(&[
        "fleets",
        "update",
        "cli-fleet",
        "--file",
        &path,
        "--if-match",
        version.as_str(),
    ]);
    let (code, updated) = fixture.cli(update.clone()).await?;
    assert_eq!(code, 0, "{updated}");
    let (code, conflict) = fixture.cli(update).await?;
    assert_eq!(code, 5, "{conflict}");
    let version = fixture
        .client
        .fleets()
        .get("cli-fleet")
        .await?
        .version
        .ok_or("version")?;
    let (code, retired) = fixture
        .cli(args(&[
            "fleets",
            "retire",
            "cli-fleet",
            "--if-match",
            version.as_str(),
            "--yes",
        ]))
        .await?;
    assert_eq!(code, 0, "{retired}");
    assert_eq!(retired["receipt"]["outcome"], "accepted");
    let pool = dir.path().join("pool.json");
    std::fs::write(
        &pool,
        serde_json::to_vec(
            &json!({"members":[{"key":"one","template_profile_ref":"k8s-linux","weight":1,"template_inputs":{}}],"failure_policy":"backpressure"}),
        )?,
    )?;
    let pool = pool.to_string_lossy();
    let (code, created) = fixture
        .cli(args(&["pools", "create", "cli-pool", "--file", &pool]))
        .await?;
    assert_eq!(code, 0, "{created}");
    assert_eq!(created["receipt"]["change"]["kind"], "template-pool");
    let resource = fixture.client.pools().get_typed("cli-pool").await?;
    let version = resource.version.ok_or("version")?;
    let (code, retired) = fixture
        .cli(args(&[
            "pools",
            "retire",
            "cli-pool",
            "--if-match",
            version.as_str(),
            "--yes",
        ]))
        .await?;
    assert_eq!(code, 0, "{retired}");
    Ok(())
}

#[tokio::test]
async fn template_revision_input_contract_attestation_and_omitted_bindings_update() -> TestResult {
    let fixture = Fixture::new().await?;
    let revision = fixture
        .client
        .templates()
        .revision_typed("k8s-linux", 1)
        .await?;
    assert_eq!(revision.data.artifact_digest, fixture.digest);
    let attestation = fixture
        .client
        .templates()
        .attestation_typed("k8s-linux", 1, "att")
        .await?;
    assert!(attestation.data.subject_verified);
    assert!(attestation.data.completed_at > 0);
    let contract = fixture
        .client
        .templates()
        .input_contract_typed("k8s-linux", 1)
        .await?;
    assert_eq!(contract.data.profile_key, "k8s-linux");
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("update.json");
    let mut body = json!({"artifact_digest":fixture.digest,"engine_ref":"terraform"});
    std::fs::write(&path, serde_json::to_vec(&body)?)?;
    let version = fixture
        .client
        .templates()
        .get("k8s-linux")
        .await?
        .version
        .ok_or("version")?;
    let update = args(&[
        "templates",
        "update",
        "k8s-linux",
        "--file",
        &path.to_string_lossy(),
        "--if-match",
        version.as_str(),
    ]);
    let (code, noop) = fixture.cli(update.clone()).await?;
    assert_eq!(code, 0, "{noop}");
    assert_eq!(noop["receipt"]["outcome"], "no_op");
    body["bindings"] = Value::Null;
    std::fs::write(&path, serde_json::to_vec(&body)?)?;
    let (code, rejected) = fixture.cli(update).await?;
    assert_eq!(code, 7, "{rejected}");
    Ok(())
}
