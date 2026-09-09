use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::Response;
use axum::Router;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};
use shaula_core::auth_context::{AccountBinding, RepositorySelection};
use shaula_core::auth_policy::AccountKind;
use shaula_core::registry::{
    auth_dependent_set_fingerprint, AuthPromotion, AuthPromotionOutcome, AuthValidationSnapshot,
    ControlPlaneStore,
};
use shaula_store::registry_impl::SqliteControlPlane;
use tower::ServiceExt;

use super::{common, history};

pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
pub const KEY: &str = "policy-app";
pub const URI: &str = "/api/v1/github-auth-profiles/policy-app";
pub const PRIVATE_KEY: &str = "policy-update-active-private-key\ncomplete-bytes";
const NOW: i64 = 1_800_000_000_000;

pub struct Fixture {
    pub app: Router,
    pub store: Arc<SqliteControlPlane>,
    pub db: DatabaseConnection,
    pub engine: std::path::PathBuf,
}

impl Fixture {
    pub async fn new() -> TestResult<Self> {
        let (app, store, engine) = common::build_app_with_scan().await;
        let fixture = Self {
            app,
            store,
            db: history::database(&engine).await?,
            engine,
        };
        let response = fixture
            .app
            .clone()
            .oneshot(common::authorized("PUT", URI, Some(full_body(PRIVATE_KEY))))
            .await?;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        assert_eq!(fixture.promote(1).await?, AuthPromotionOutcome::Promoted);
        Ok(fixture)
    }

    pub async fn etag(&self) -> TestResult<String> {
        let response = self
            .app
            .clone()
            .oneshot(common::authorized("GET", URI, None))
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        Ok(response
            .headers()
            .get("etag")
            .ok_or("missing ETag")?
            .to_str()?
            .into())
    }

    pub async fn post(&self, body: String, etag: &str, idem: &str) -> TestResult<Response> {
        let mut request = common::authorized("POST", &format!("{URI}/policy-updates"), Some(body));
        request.headers_mut().insert("if-match", etag.parse()?);
        request
            .headers_mut()
            .insert("idempotency-key", idem.parse()?);
        Ok(self.app.clone().oneshot(request).await?)
    }

    pub async fn rotate(&self, key: &str, idem: &str) -> TestResult {
        let mut request = common::authorized("PUT", URI, Some(full_body(key)));
        request.headers_mut().remove("if-none-match");
        request
            .headers_mut()
            .insert("if-match", self.etag().await?.parse()?);
        request
            .headers_mut()
            .insert("idempotency-key", idem.parse()?);
        assert_eq!(
            self.app.clone().oneshot(request).await?.status(),
            StatusCode::ACCEPTED
        );
        Ok(())
    }

    pub async fn promote(&self, revision: i64) -> TestResult<AuthPromotionOutcome> {
        let snapshot = AuthValidationSnapshot {
            candidate: (KEY.into(), revision),
            dependent_set: auth_dependent_set_fingerprint(&[]),
            checked_fleets: vec![],
            identities: vec![],
        };
        Ok(self
            .store
            .auth_apply_validation_v2(
                KEY,
                revision,
                true,
                None,
                NOW,
                Some(AuthPromotion {
                    bindings: vec![AccountBinding {
                        account_id: 100,
                        account_kind: AccountKind::Organization,
                        login: "example-org".into(),
                        installation_id: 11,
                        repository_selection: RepositorySelection::All,
                        validated_at_ms: NOW,
                    }],
                    snapshot_json: serde_json::to_string(&snapshot)?,
                }),
            )
            .await?)
    }

    pub async fn execute(&self, sql: &str) -> TestResult {
        self.db.execute_unprepared(sql).await?;
        Ok(())
    }

    pub async fn counts(&self) -> TestResult<[i64; 5]> {
        let mut counts = [0; 5];
        for (index, table) in [
            "github_auth_profile_revisions",
            "profile_changes",
            "audit_records",
            "outbox",
            "idempotency_records",
        ]
        .iter()
        .enumerate()
        {
            let result = self
                .db
                .query_one(Statement::from_string(
                    DatabaseBackend::Sqlite,
                    format!("SELECT COUNT(*) AS count FROM {table}"),
                ))
                .await?
                .ok_or("count missing")?;
            counts[index] = result.try_get("", "count")?;
        }
        Ok(counts)
    }
}

pub fn body(base_revision: i64, owners: &[&str]) -> String {
    serde_json::json!({
        "base_revision": base_revision,
        "target_policy": owners.iter().map(|owner| serde_json::json!({"kind": "organization", "owner": owner})).collect::<Vec<_>>(),
    }).to_string()
}

pub fn full_body(private_key: &str) -> String {
    serde_json::json!({
        "kind": "github_app", "schema_version": 2, "app_id": "4863460",
        "private_key": private_key,
        "target_policy": [{"kind": "organization", "owner": "example-org"}],
    })
    .to_string()
}

pub async fn json_response(response: Response) -> TestResult<serde_json::Value> {
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await?;
    Ok(serde_json::from_slice(&bytes)?)
}
