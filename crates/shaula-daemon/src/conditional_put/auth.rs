//! Auth publication shares protocol, not provider identity or validation rules.

use async_trait::async_trait;
use shaula_core::auth::AuthKind;
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::registry::{
    AuthProfilePut, AuthRevisionRow, MutationAccepted, MutationError, ProfileHead, Scope,
};

use super::request::{internal, Commit, Identity, Plan};
use super::{ControlPlane, Outcome, Resource};
use crate::service::unprocessable;
use crate::service_auth_format::{AuthPutFormat, ForgejoAuthPutFormat};

enum Format {
    Github(AuthPutFormat),
    Forgejo(ForgejoAuthPutFormat),
}

pub(in crate::service) struct Auth {
    payload: AuthProfilePut,
    format: Format,
    canonical: String,
}

impl Auth {
    pub(in crate::service) fn prepare(key: &str, payload: AuthProfilePut) -> Outcome<Self> {
        if let Err(error) = shaula_core::auth::validate_profile_key_for_fleet_ref(key) {
            return Ok(Err(unprocessable(error.code, error.summary)));
        }
        // Unsupported formats are rejected BEFORE looking for historical replay.
        let format = if payload.kind == AuthKind::ForgejoToken {
            ForgejoAuthPutFormat::parse(&payload).map(Format::Forgejo)
        } else {
            AuthPutFormat::parse(&payload).map(Format::Github)
        };
        let format = match format {
            Ok(format) => format,
            Err((reason, summary)) => return Ok(Err(unprocessable(reason, summary))),
        };
        let canonical = match &format {
            Format::Github(format) => format.canonical_body(),
            Format::Forgejo(format) => format.canonical_body(),
        };
        Ok(Ok(Self {
            payload,
            format,
            canonical,
        }))
    }

    async fn github_change_kind(
        &self,
        plane: &ControlPlane,
        key: &str,
        current: Option<&ProfileHead>,
        format: &AuthPutFormat,
    ) -> Outcome<&'static str> {
        let mut head_policy = None;
        if let Some(current) = current {
            let Some(head) = plane
                .store
                .auth_revision_get(key, current.desired_revision)
                .await?
            else {
                return Ok(Err(MutationError::IdentityConflict));
            };
            if head.schema_version != 2 || head.kind != "github_app" {
                return Ok(Err(unprocessable(
                    ReasonCode::SpecInvalid,
                    "unsupported authentication profile; publish under a new profile key",
                )));
            }
            head_policy = head.policy_json;
            if let Some(revision) = current.active_revision {
                let Some(active) = plane.store.auth_revision_get(key, revision).await? else {
                    return Ok(Err(MutationError::IdentityConflict));
                };
                if active.schema_version != 2 || active.kind != "github_app" {
                    return Ok(Err(unprocessable(
                        ReasonCode::SpecInvalid,
                        "unsupported authentication profile; publish under a new profile key",
                    )));
                }
                if active.app_id.as_deref() != Some(format.app_id.as_str()) {
                    return Ok(Err(MutationError::IdentityConflict));
                }
            }
        }
        Ok(Ok(
            if head_policy.as_deref() == Some(format.policy_json.as_str()) {
                "Rotate"
            } else {
                "Publish"
            },
        ))
    }

    async fn forgejo_identity(
        &self,
        plane: &ControlPlane,
        key: &str,
        current: Option<&ProfileHead>,
        format: &ForgejoAuthPutFormat,
    ) -> Outcome<()> {
        if let Some(current) = current {
            let Some(head) = plane
                .store
                .auth_revision_get(key, current.desired_revision)
                .await?
            else {
                return Ok(Err(MutationError::IdentityConflict));
            };
            if head.kind != "forgejo_token" || head.schema_version != 1 {
                return Ok(Err(MutationError::IdentityConflict));
            }
            let target = head
                .policy_json
                .as_deref()
                .ok_or_else(|| internal("Forgejo target is missing"))?;
            let target: shaula_core::forgejo::ForgejoTarget = serde_json::from_str(target)
                .map_err(|error| {
                    CoreError::new(
                        ReasonCode::Internal,
                        format!("Forgejo target is invalid: {error}"),
                    )
                })?;
            if target != format.target {
                return Ok(Err(MutationError::IdentityConflict));
            }
        }
        Ok(Ok(()))
    }
}

#[async_trait]
impl Resource for Auth {
    type Head = ProfileHead;
    type Facts = ();
    const SCOPE: Scope = Scope::AuthWrite;

    fn identity(&self) -> Identity<'_> {
        Identity {
            kind: match self.format {
                Format::Github(_) => "github_auth_profile",
                Format::Forgejo(_) => "forgejo_auth_profile",
            },
            canonical: &self.canonical,
            includes_precondition: false,
        }
    }
    async fn current(&self, plane: &ControlPlane, key: &str) -> CoreResult<Option<ProfileHead>> {
        plane.store.auth_profile_get(key).await
    }
    async fn replay_matches(
        &self,
        plane: &ControlPlane,
        key: &str,
        accepted: &MutationAccepted,
    ) -> CoreResult<bool> {
        let stored = plane
            .store
            .auth_credential_bytes(key, accepted.change.revision)
            .await?
            .unwrap_or_default();
        Ok(constant_time_eq(
            &stored,
            self.payload.secret.expose().as_bytes(),
        ))
    }
    async fn admit(
        &self,
        plane: &ControlPlane,
        key: &str,
        head: Option<&ProfileHead>,
    ) -> Outcome<Plan<Self::Facts>> {
        let kind = match &self.format {
            Format::Github(format) => {
                match self.github_change_kind(plane, key, head, format).await? {
                    Ok(kind) => kind,
                    Err(error) => return Ok(Err(error)),
                }
            }
            Format::Forgejo(format) => {
                if let Err(error) = self.forgejo_identity(plane, key, head, format).await? {
                    return Ok(Err(error));
                }
                if head.is_some() {
                    "Rotate"
                } else {
                    "Publish"
                }
            }
        };
        // Deliberately preserve the existing Candidate/revalidation behaviour.
        // Auth no-op versus explicit revalidation is a separate contract decision.
        Ok(Ok(Plan::Revision { kind, facts: () }))
    }
    async fn commit(
        &self,
        plane: &ControlPlane,
        commit: Commit<'_>,
        admitted: Option<Self::Facts>,
    ) -> Outcome<()> {
        admitted.ok_or_else(|| internal("Auth publication requires a Candidate"))?;
        let (app_id, policy_json, kind, schema_version) = match &self.format {
            Format::Github(format) => (
                Some(format.app_id.clone()),
                format.policy_json.clone(),
                "github_app",
                2,
            ),
            Format::Forgejo(format) => (None, format.target_json.clone(), "forgejo_token", 1),
        };
        let row = AuthRevisionRow {
            profile_key: commit.key.into(),
            revision: commit.revision(),
            state: "Validating".into(),
            reason: None,
            kind: kind.into(),
            app_id,
            schema_version,
            policy_json: Some(policy_json),
            validation_snapshot_json: None,
        };
        let facts = commit.facts(
            self.identity().kind,
            "profile.auth_validate",
            format!(
                "{{\"key\":\"{}\",\"revision\":{}}}",
                commit.key,
                commit.revision()
            ),
        );
        plane
            .store
            .commit_auth_revision(facts, row, self.payload.secret.expose().as_bytes())
            .await
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (left, right) in a.iter().zip(b.iter()) {
        diff |= left ^ right;
    }
    diff == 0
}
