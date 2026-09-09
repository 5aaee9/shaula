//! Real HTTP/OIDC, SQLite and local GitHub fixtures for installation links.

use super::*;
use axum::body::{to_bytes, Body};
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use sea_orm::{ConnectionTrait, Statement};
use shaula_core::registry::ControlPlaneStore;
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;

pub(super) use crate::http_oidc as oidc;

pub(super) type TestResult = Result<(), Box<dyn std::error::Error>>;
pub(super) const KEY: &str = crate::auth_worker_v2::tests::KEY;
pub(super) const URI: &str = "/api/v1/github-auth-profiles/shared-github/installation-link";

#[derive(Clone)]
struct GitHub {
    state: Arc<tokio::sync::Mutex<GitHubState>>,
    calls: Arc<AtomicUsize>,
    db: sea_orm::DatabaseConnection,
}

struct GitHubState {
    status: StatusCode,
    body: String,
    sql: Option<String>,
}

async fn github_app(State(github): State<GitHub>) -> axum::response::Response {
    github.calls.fetch_add(1, Ordering::SeqCst);
    let state = github.state.lock().await;
    if let Some(sql) = &state.sql {
        if github
            .db
            .execute(Statement::from_string(sea_orm::DbBackend::Sqlite, sql))
            .await
            .is_err()
        {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
    (state.status, state.body.clone()).into_response()
}

struct NoPublisher;
#[async_trait::async_trait]
impl shaula_http::router::ArtifactPublisher for NoPublisher {
    async fn publish(&self, _: &[u8], _: &str) -> shaula_core::error::CoreResult<u64> {
        Err(shaula_core::error::CoreError::new(
            shaula_core::error::ReasonCode::Internal,
            "artifact publishing is unavailable in this fixture",
        ))
    }
}

pub(super) struct Fixture {
    pub plane: crate::wiring::wiring_tests::TestPlane,
    pub app: axum::Router,
    pub links: Arc<StoredAuthInstallationLink>,
    github: GitHub,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Fixture {
    pub async fn new(wired: bool) -> Result<Self, Box<dyn std::error::Error>> {
        let plane = crate::wiring::wiring_tests::test_plane().await;
        let db = sea_orm::Database::connect(format!(
            "sqlite:{}?mode=rw",
            plane.db_path.display().to_string().replace('\\', "/"),
        ))
        .await?;
        let github = GitHub {
            state: Arc::new(tokio::sync::Mutex::new(GitHubState {
                status: StatusCode::OK,
                body: r#"{"id":4863460,"slug":"shaula-fixture","html_url":"https://evil.test/"}"#
                    .into(),
                sql: None,
            })),
            calls: Arc::new(AtomicUsize::new(0)),
            db,
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let router = axum::Router::new()
            .route("/app", get(github_app))
            .with_state(github.clone());
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        let clock: Arc<dyn Clock> = Arc::new(crate::auth_worker_mock::Now);
        let links = Arc::new(StoredAuthInstallationLink {
            store: plane.control_plane.clone(),
            resolver: AppInstallationResolver::new(
                base,
                shaula_scaleset::client::production_http_client()?,
                clock.clone(),
            ),
        });
        let service = Arc::new(shaula_daemon::service::ControlPlane::new(
            plane.control_plane.clone(),
            clock,
            b"installation-link-fixture".to_vec(),
            100,
            "target/unused-engine".into(),
        ));
        let app = shaula_http::router::build_router(shaula_http::router::AppState {
            fleets: service.clone(),
            profiles: service.clone(),
            health: service,
            oidc: oidc::provider().oidc().await,
            body_limit: 64 * 1024,
            request_body_limit: 64 * 1024,
            jobs: None,
            logs: None,
            artifact_publisher: Arc::new(NoPublisher),
            auth_installation_link: wired
                .then(|| links.clone() as Arc<dyn AuthInstallationLinkPort>),
        });
        Ok(Self {
            plane,
            app,
            links,
            github,
            server,
        })
    }

    pub fn calls(&self) -> usize {
        self.github.calls.load(Ordering::SeqCst)
    }

    pub async fn seed(&self, active: bool) -> TestResult {
        crate::auth_worker_v2::tests::seed_candidate(
            &self.plane.control_plane,
            1,
            &crate::auth_worker_v2::tests::policy(false),
        )
        .await;
        if active {
            self.plane
                .control_plane
                .auth_apply_validation_v2(
                    KEY,
                    1,
                    true,
                    None,
                    1_800_000_000_000,
                    Some(crate::auth_worker_mock::promotion_from(KEY, 1, &[])),
                )
                .await?;
        }
        Ok(())
    }

    pub async fn sql(&self, sql: &str) -> TestResult {
        self.github
            .db
            .execute(Statement::from_string(sea_orm::DbBackend::Sqlite, sql))
            .await?;
        Ok(())
    }

    pub async fn sql_during_lookup(&self, sql: &str) {
        self.github.state.lock().await.sql = Some(sql.into());
    }

    pub async fn reply(&self, status: StatusCode, body: &str) {
        let mut state = self.github.state.lock().await;
        state.status = status;
        state.body = body.into();
    }

    pub async fn get(
        &self,
        uri: &str,
        scopes: Option<&str>,
    ) -> Result<(StatusCode, serde_json::Value), Box<dyn std::error::Error>> {
        let mut request = Request::builder().uri(uri);
        if let Some(scopes) = scopes {
            request = request.header("authorization", oidc::bearer(scopes));
        }
        let response = self
            .app
            .clone()
            .oneshot(request.body(Body::empty())?)
            .await?;
        let status = response.status();
        if scopes.is_some() && uri == URI {
            assert!(response.headers()["cache-control"]
                .to_str()?
                .contains("no-store"));
        }
        let bytes = to_bytes(response.into_body(), 65536).await?;
        let text = std::str::from_utf8(&bytes)?;
        assert!(!text.contains("PRIVATE KEY") && !text.contains("eyJ"));
        Ok((status, serde_json::from_slice(&bytes)?))
    }

    pub async fn table_counts(&self) -> Result<Vec<i64>, Box<dyn std::error::Error>> {
        let db = &self.github.db;
        let tables = db
            .query_all(Statement::from_string(
                sea_orm::DbBackend::Sqlite,
                "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name".to_owned(),
            ))
            .await?;
        let mut counts = Vec::new();
        for table in tables {
            let name: String = table.try_get("", "name")?;
            let row = db
                .query_one(Statement::from_string(
                    sea_orm::DbBackend::Sqlite,
                    format!(
                        "SELECT count(*) AS count FROM \"{}\"",
                        name.replace('"', "\"\"")
                    ),
                ))
                .await?
                .ok_or("count missing")?;
            counts.push(row.try_get("", "count")?);
        }
        Ok(counts)
    }
}
