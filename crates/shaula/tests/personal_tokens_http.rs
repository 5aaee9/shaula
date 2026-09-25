#[path = "support/personal_token_cli.rs"]
mod cli_acceptance;
#[path = "../../shaula-http/tests/support/mod.rs"]
mod oidc;
#[path = "support/personal_token_routes.rs"]
mod route_acceptance;
#[path = "support/personal_token_workflows.rs"]
mod workflows;
use axum::http::StatusCode;
use shaula_client::{
    types::{Document, IssueToken, TokenIssue},
    Client, MutationOptions, Secret,
};
use shaula_core::{access_tokens::TokenPolicy, registry::Scope};
use shaula_daemon::{access_tokens::PersonalTokens, service::ControlPlane};
use shaula_store::{registry_impl::SqliteControlPlane, Store};
use std::sync::Arc;
type TestResult = Result<(), Box<dyn std::error::Error>>;
struct Time;
impl shaula_core::ports::Clock for Time {
    fn now_unix_ms(&self) -> i64 {
        1_800_000_000_000
    }
}
struct Publisher;
#[async_trait::async_trait]
impl shaula_http::router::ArtifactPublisher for Publisher {
    async fn publish(&self, _: &[u8], _: &str) -> shaula_core::CoreResult<u64> {
        Ok(0)
    }
}
struct Fixture {
    _dir: tempfile::TempDir,
    origin: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let store = Store::open(&dir.path().join("db")).await?;
        store.migrate().await?;
        let scopes = Scope::ALL
            .iter()
            .map(|s| s.as_str().to_owned())
            .collect::<Vec<_>>();
        let provider = oidc::provider();
        let config = shaula_http::oidc::OidcConfig::new(
            provider.issuer.clone(),
            "web".into(),
            shaula_core::secret::SecretString::new("test-secret"),
            "https://shaula.example".into(),
            "api".into(),
            ["ops", "other"]
                .map(|subject| shaula_http::oidc::Grant {
                    issuer: provider.issuer.clone(),
                    subject: subject.into(),
                    scopes: scopes.clone(),
                })
                .into(),
        )?;
        let oidc = shaula_http::oidc::Oidc::discover_with_roots(
            config,
            vec![reqwest::Certificate::from_pem(&provider.certificate)?],
        )
        .await?;
        let (realm, grants) = oidc.token_context();
        let tokens = PersonalTokens::new(
            Arc::new(store.clone()),
            Arc::new(Time),
            TokenPolicy::default(),
            realm,
            grants,
        )?;
        let control = Arc::new(ControlPlane::new(
            Arc::new(SqliteControlPlane::new(
                store.clone(),
                dir.path().join("artifacts"),
            )),
            Arc::new(Time),
            vec![42; 32],
            100,
            dir.path().join("engine"),
        ));
        control.set_ready(true);
        let app = shaula_http::router::build_router(shaula_http::router::AppState {
            access_tokens: Some(Arc::new(tokens)),
            fleets: control.clone(),
            profiles: control.clone(),
            pools: control.clone(),
            health: control,
            oidc,
            body_limit: 1 << 20,
            request_body_limit: 1 << 20,
            artifact_publisher: Arc::new(Publisher),
            auth_installation_link: None,
            jobs: Some(Arc::new(store.clone())),
            logs: None,
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self {
            _dir: dir,
            origin,
            task,
        })
    }
    fn primary(&self, subject: &str) -> Result<Client, shaula_client::Error> {
        let scopes = Scope::ALL
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let raw = oidc::sign(
            oidc::claims(&oidc::provider().issuer, "api", subject, &scopes),
            "at+jwt",
            "test-key",
        );
        Client::loopback(&self.origin, Secret::new(raw))
    }
}
fn request(scopes: Vec<String>) -> IssueToken {
    IssueToken {
        name: "test token".into(),
        scopes,
        expires_in_seconds: Some(3600),
    }
}

#[tokio::test]
async fn real_http_issue_replay_guard_mutation_audit_and_self_revoke() -> TestResult {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    let fixture = Fixture::new().await?;
    let primary = fixture.primary("ops")?;
    let issue = primary
        .access_tokens()
        .issue(
            &request(vec![
                "auth.write".into(),
                "auth.read".into(),
                "fleet.read".into(),
            ]),
            "issue-key".into(),
        )
        .await?;
    let TokenIssue::Issued { metadata, secret } = issue else {
        return Err("expected one-time secret".into());
    };
    let pat = Client::loopback(&fixture.origin, secret.clone())?;
    let session = pat.session().await?.data;
    assert_eq!(session.principal, primary.session().await?.data.principal);
    assert_eq!(
        session.authentication.ok_or("provenance")?.kind,
        "personal_access_token"
    );
    let replay = primary
        .access_tokens()
        .issue(
            &request(vec![
                "auth.write".into(),
                "auth.read".into(),
                "fleet.read".into(),
            ]),
            "issue-key".into(),
        )
        .await?;
    assert!(matches!(replay, TokenIssue::RecoveredWithoutSecret(_)));
    assert_eq!(replay.metadata().id, metadata.id);
    assert!(matches!(
        pat.access_tokens()
            .issue(&request(vec![]), "no".into())
            .await,
        Err(shaula_client::Error::Http { status: 403, .. })
    ));
    assert!(matches!(
        fixture
            .primary("other")?
            .access_tokens()
            .get(&metadata.id)
            .await,
        Err(shaula_client::Error::Http { status: 404, .. })
    ));
    let body=Document::parse(r#"{"kind":"github_app","schema_version":2,"app_id":"42","private_key":"test-private-key","target_policy":[{"kind":"organization","owner":"example"}]}"#.into())?;
    let mut options = MutationOptions::create();
    options.idempotency_key = "shared-key".into();
    let attempt = pat
        .auth_profiles()
        .put("test-app", &body, options.clone())
        .await?;
    let result = pat.execute_mutation(&attempt).await?;
    assert_eq!(result.data.state, "Pending");
    let profile = pat.auth_profiles().get_typed("test-app").await?;
    let mut changed_precondition =
        MutationOptions::update(profile.version.clone().ok_or("version")?);
    changed_precondition.idempotency_key = "shared-key".into();
    let changed = pat
        .auth_profiles()
        .put("test-app", &body, changed_precondition)
        .await?;
    assert!(matches!(
        pat.execute_mutation(&changed).await,
        Err(shaula_client::Error::Http { status: 409, .. })
    ));
    assert_eq!(profile.data.key, "test-app");
    assert_eq!(profile.data.schema_version, Some(2));
    assert!(profile.version.is_some());
    assert!(!profile.raw.raw().contains("test-private-key"));
    let revision = pat.auth_profiles().revision_typed("test-app", 1).await?;
    assert_eq!(revision.data.app_id.as_deref(), Some("42"));
    assert_eq!(revision.data.schema_version, 2);
    assert_eq!(
        pat.auth_profiles().list_typed().await?.data.profiles.len(),
        1
    );
    assert_eq!(
        pat.auth_profiles()
            .status_typed("test-app")
            .await?
            .data
            .desired_revision,
        1
    );
    assert!(pat
        .auth_profiles()
        .impact_typed("test-app")
        .await?
        .data
        .live_fleets
        .is_empty());
    let rotated = primary
        .auth_profiles()
        .put("test-app", &body, options.clone())
        .await?;
    assert_eq!(
        primary.execute_mutation(&rotated).await?.data.change_id,
        result.data.change_id
    );
    let other = fixture.primary("other")?;
    let attempt = other
        .auth_profiles()
        .put("test-app", &body, options)
        .await?;
    assert!(matches!(
        other.execute_mutation(&attempt).await,
        Err(shaula_client::Error::Http { status: 412, .. })
    ));
    let mut options = sea_orm::ConnectOptions::new(format!(
        "sqlite://{}",
        fixture
            ._dir
            .path()
            .join("db")
            .to_string_lossy()
            .replace('\\', "/")
    ));
    options.sqlx_logging(false);
    let db = sea_orm::Database::connect(options).await?;
    let row=db.query_one(Statement::from_string(DatabaseBackend::Sqlite,"SELECT actor,credential_kind,access_token_id FROM audit_records WHERE resource_key='test-app'".to_owned())).await?.ok_or("audit")?;
    assert_eq!(
        row.try_get::<String>("", "credential_kind")?,
        "personal_access_token"
    );
    assert_eq!(row.try_get::<String>("", "access_token_id")?, metadata.id);
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    for path in ["/", "/assets/no.js", "/auth/oidc/logout"] {
        let result = http
            .get(format!("{}{path}", fixture.origin))
            .bearer_auth(secret.expose())
            .send()
            .await?;
        assert_eq!(result.status(), StatusCode::UNAUTHORIZED);
    }
    let mixed = http
        .get(format!("{}/api/v1/session", fixture.origin))
        .bearer_auth(secret.expose())
        .header("cookie", "__Host-shaula-session=other")
        .send()
        .await?;
    assert_eq!(mixed.status(), StatusCode::UNAUTHORIZED);
    pat.access_tokens().revoke_current().await?;
    assert!(matches!(
        pat.session().await,
        Err(shaula_client::Error::Http { status: 401, .. })
    ));
    Ok(())
}

#[tokio::test]
async fn cli_uses_http_without_daemon_bootstrap_and_redacts_default_output() -> TestResult {
    let fixture = Fixture::new().await?;
    let issue = fixture
        .primary("ops")?
        .access_tokens()
        .issue(&request(vec!["fleet.read".into()]), "cli".into())
        .await?;
    let TokenIssue::Issued { secret, .. } = issue else {
        return Err("secret".into());
    };
    let origin = fixture.origin.clone();
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new(env!("CARGO_BIN_EXE_shaula"))
            .args([
                "--server",
                &origin,
                "--allow-loopback-http",
                "--output",
                "json",
                "fleets",
                "list",
            ])
            .env("SHAULA_ACCESS_TOKEN", secret.expose())
            .output()
    })
    .await??;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(json["schema_version"], 1);
    assert!(json["error"].is_null());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("shaula_pat_v1_"));
    Ok(())
}
