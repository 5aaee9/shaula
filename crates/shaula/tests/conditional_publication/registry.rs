use std::sync::Arc;

use axum::Router;
use sea_orm::{ConnectionTrait, Database, DatabaseBackend, DatabaseConnection, Statement};
use shaula_core::auth::AuthKind;
use shaula_core::registry::{
    Actor, AuthProfilePut, ControlPlaneStore, FleetRegistryPort, MutationAccepted, MutationError,
    ProfileRegistryPort, Scope, TemplatePoolRegistryPort, TemplateProfilePut,
};
use shaula_core::secret::SecretString;
use shaula_daemon::service::ControlPlane;
use shaula_store::registry_impl::SqliteControlPlane;

use super::{common, support::*};

pub struct Fixture {
    pub app: Router,
    pub store: Arc<SqliteControlPlane>,
    pub service: Arc<ControlPlane>,
    pub engine: std::path::PathBuf,
    pub digest: String,
    pub db: DatabaseConnection,
}

impl Fixture {
    pub async fn new() -> TestResult<Self> {
        let (app, store, engine, service) = common::build_app_with_service().await;
        let digest =
            common::attestation_harness::seed_profile(&app, &store, "k8s-linux", true).await;
        let root = engine
            .parent()
            .and_then(std::path::Path::parent)
            .ok_or("fixture root")?;
        let db = Database::connect(format!(
            "sqlite://{}?mode=rw",
            root.join("data/test.db").display()
        ))
        .await?;
        Ok(Self {
            app,
            store,
            service,
            engine,
            digest,
            db,
        })
    }

    pub async fn count(&self, table: &str) -> TestResult<i64> {
        let row = self
            .db
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                format!("SELECT COUNT(*) AS count FROM {table}"),
            ))
            .await?
            .ok_or("count missing")?;
        Ok(row.try_get("", "count")?)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Kind {
    Fleet,
    Pool,
    Template,
    Github,
    Forgejo,
}

pub const KINDS: [Kind; 5] = [
    Kind::Fleet,
    Kind::Pool,
    Kind::Template,
    Kind::Github,
    Kind::Forgejo,
];

#[derive(Clone)]
pub struct Conditions {
    pub create: bool,
    pub expected: Option<(String, i64)>,
    pub idem: Option<String>,
}

impl Conditions {
    pub fn create() -> Self {
        Self {
            create: true,
            expected: None,
            idem: None,
        }
    }
    pub fn replace(accepted: &MutationAccepted, idem: &str) -> TestResult<Self> {
        let (incarnation, revision) = accepted.etag.rsplit_once(':').ok_or("ETag")?;
        Ok(Self {
            create: false,
            expected: Some((incarnation.into(), revision.parse()?)),
            idem: Some(idem.into()),
        })
    }
}

pub fn actor() -> Actor {
    Actor {
        authentication: Default::default(),
        name: "publisher".into(),
        scopes: vec![Scope::FleetWrite, Scope::TemplatePublish, Scope::AuthWrite],
    }
}

impl Kind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Fleet => "publication-fleet",
            Self::Pool => "publication-pool",
            Self::Template => "publication-template",
            Self::Github => "publication-github",
            Self::Forgejo => "publication-forgejo",
        }
    }
    pub fn resource_kind(self) -> &'static str {
        match self {
            Self::Fleet => "fleet",
            Self::Pool => "template_pool",
            Self::Template => "template_profile",
            Self::Github => "github_auth_profile",
            Self::Forgejo => "forgejo_auth_profile",
        }
    }
    pub fn uri(self) -> String {
        let collection = match self {
            Self::Fleet => "fleets",
            Self::Pool => "template-pools",
            Self::Template => "template-profiles",
            Self::Github | Self::Forgejo => "github-auth-profiles",
        };
        format!("/api/v1/{collection}/{}", self.key())
    }
    pub fn body(self, digest: &str, variant: u8) -> TestResult<serde_json::Value> {
        let mut body: serde_json::Value = match self {
            Self::Fleet => serde_json::from_str(common::FLEET_BODY)?,
            Self::Pool => serde_json::from_str(POOL_BODY)?,
            Self::Template => {
                serde_json::from_str(&common::TEMPLATE_PUT_BODY.replace("PLACEHOLDER", digest))?
            }
            Self::Github => serde_json::from_str(common::AUTH_PUT_BODY)?,
            Self::Forgejo => serde_json::json!({
                "kind":"forgejo_token", "instance_url":"https://forgejo.example.test",
                "scope":{"kind":"instance"}, "token":"private-forgejo-token"
            }),
        };
        if variant != 0 {
            match self {
                Self::Fleet => body["capacity"]["max_runners"] = (5 + variant).into(),
                Self::Pool => body["members"][0]["weight"] = (20 + variant).into(),
                Self::Template => {
                    body["bindings"]["kubeconfig"] = format!("rotated-kubeconfig-{variant}").into()
                }
                Self::Github => {
                    body["private_key"] = format!("rotated-private-key-{variant}").into()
                }
                Self::Forgejo => body["token"] = format!("rotated-token-{variant}").into(),
            }
        }
        Ok(body)
    }

    pub async fn publish(
        self,
        plane: &ControlPlane,
        actor: &Actor,
        digest: &str,
        variant: u8,
        conditions: Conditions,
    ) -> TestResult<Result<MutationAccepted, MutationError>> {
        let body = self.body(digest, variant)?;
        let Conditions {
            create,
            expected,
            idem,
        } = conditions;
        Ok(match self {
            Self::Fleet => {
                plane
                    .fleet_put(
                        actor,
                        self.key(),
                        serde_json::from_value(body)?,
                        create,
                        expected,
                        idem,
                    )
                    .await?
            }
            Self::Pool => {
                plane
                    .template_pool_put(
                        actor,
                        self.key(),
                        serde_json::from_value(body)?,
                        create,
                        expected,
                        idem,
                    )
                    .await?
            }
            Self::Template => {
                plane
                    .template_put(
                        actor,
                        self.key(),
                        TemplateProfilePut {
                            source_key: None,
                            artifact_digest: digest.into(),
                            engine_ref: "terraform".into(),
                            bindings: body["bindings"].clone(),
                            fleet_input_policy: body["fleet_input_policy"].clone(),
                        },
                        create,
                        expected,
                        idem,
                    )
                    .await?
            }
            Self::Github | Self::Forgejo => {
                let github = matches!(self, Self::Github);
                plane
                    .auth_put(
                        actor,
                        self.key(),
                        AuthProfilePut {
                            kind: if github {
                                AuthKind::GithubApp
                            } else {
                                AuthKind::ForgejoToken
                            },
                            app_id: github.then(|| "4863460".into()),
                            secret: SecretString::new(
                                body[if github { "private_key" } else { "token" }]
                                    .as_str()
                                    .ok_or("credential")?,
                            ),
                            schema_version: Some(if github { 2 } else { 1 }),
                            target_policy: if github {
                                Some(serde_json::from_value(body["target_policy"].clone())?)
                            } else {
                                None
                            },
                            forgejo_target: if github {
                                None
                            } else {
                                Some(serde_json::from_value(serde_json::json!({
                                    "instance_url": body["instance_url"], "scope": body["scope"],
                                }))?)
                            },
                        },
                        create,
                        expected,
                        idem,
                    )
                    .await?
            }
        })
    }

    pub async fn revision(self, fixture: &Fixture) -> TestResult<i64> {
        Ok(match self {
            Self::Fleet => {
                fixture
                    .store
                    .fleet_get(self.key())
                    .await?
                    .ok_or("Fleet head")?
                    .desired_revision
            }
            Self::Pool => {
                fixture
                    .store
                    .template_pool_get(self.key())
                    .await?
                    .ok_or("Pool head")?
                    .desired_revision
            }
            Self::Template => {
                fixture
                    .store
                    .template_profile_get(self.key())
                    .await?
                    .ok_or("Template head")?
                    .desired_revision
            }
            Self::Github | Self::Forgejo => {
                fixture
                    .store
                    .auth_profile_get(self.key())
                    .await?
                    .ok_or("Auth head")?
                    .desired_revision
            }
        })
    }
}

pub fn success(outcome: Result<MutationAccepted, MutationError>) -> TestResult<MutationAccepted> {
    outcome.map_err(|error| format!("publication rejected: {error:?}").into())
}
