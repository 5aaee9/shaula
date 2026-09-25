//! Template-specific admission and immutable bindings remain behind publication.

use async_trait::async_trait;
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::registry::{MutationAccepted, MutationError, ProfileHead, Scope};

use super::request::{Commit, Identity, Plan};
use super::{ControlPlane, Outcome, Resource};
use crate::service::{
    profile_update::TemplatePublication, shaula_template_manifest, unprocessable,
};

pub(in crate::service) struct Template {
    publication: TemplatePublication,
    canonical: String,
    bindings_json: String,
    policy_json: String,
}

impl Template {
    pub(in crate::service) fn prepare(
        key: &str,
        publication: TemplatePublication,
    ) -> Outcome<Self> {
        if let Err(error) = shaula_core::auth::validate_profile_key_for_fleet_ref(key) {
            return Ok(Err(unprocessable(error.code, error.summary)));
        }
        let canonical = publication.canonical()?;
        let bindings_json = serde_json::to_string(&publication.payload.bindings)
            .map_err(|error| CoreError::new(ReasonCode::Internal, error.to_string()))?;
        let policy_json = serde_json::to_string(&publication.payload.fleet_input_policy)
            .map_err(|error| CoreError::new(ReasonCode::Internal, error.to_string()))?;
        Ok(Ok(Self {
            publication,
            canonical,
            bindings_json,
            policy_json,
        }))
    }
}

#[async_trait]
impl Resource for Template {
    type Head = ProfileHead;
    type Facts = String;
    const SCOPE: Scope = Scope::TemplatePublish;

    fn identity(&self) -> Identity<'_> {
        Identity {
            kind: "template_profile",
            canonical: &self.canonical,
            includes_precondition: false,
        }
    }
    async fn current(&self, plane: &ControlPlane, key: &str) -> CoreResult<Option<ProfileHead>> {
        plane.store.template_profile_get(key).await
    }
    async fn replay_matches(
        &self,
        plane: &ControlPlane,
        key: &str,
        accepted: &MutationAccepted,
    ) -> CoreResult<bool> {
        let stored = plane
            .store
            .template_protected_bindings(key, accepted.change.revision)
            .await?
            .map(|(json, _)| json)
            .unwrap_or_default();
        Ok(stored == self.bindings_json)
    }
    async fn admit(
        &self,
        plane: &ControlPlane,
        key: &str,
        head: Option<&ProfileHead>,
    ) -> Outcome<Plan<String>> {
        let payload = &self.publication.payload;
        // Only NEW publications consult today's artifact/source policy. The
        // protocol has already resolved any accepted historical response.
        let Some(manifest_yaml) = plane
            .store
            .artifact_manifest(&payload.artifact_digest)
            .await?
        else {
            return Ok(Err(unprocessable(
                ReasonCode::TemplateInvalid,
                "artifact digest is not published",
            )));
        };
        let manifest = shaula_template_manifest(&manifest_yaml)?;
        if let Err(error) = plane
            .validate_template_source(
                payload,
                &manifest,
                self.publication.update_base_source.as_deref(),
            )
            .await?
        {
            return Ok(Err(error));
        }
        if !plane
            .store
            .artifact_shape_ok(&payload.artifact_digest)
            .await?
        {
            return Ok(Err(unprocessable(
                ReasonCode::TemplateInvalid,
                "artifact shape rejected",
            )));
        }
        // The incarnation FOUNDER owns platform/contract identity, not a
        // pending desired Candidate that may not yet have scan metadata.
        if head.is_some() {
            let founder = plane
                .store
                .template_revision_get(key, 1)
                .await?
                .ok_or_else(|| {
                    CoreError::new(
                        ReasonCode::Internal,
                        "incarnation founder revision row missing",
                    )
                })?;
            let founder_text = plane
                .store
                .artifact_manifest(&founder.artifact_digest)
                .await?
                .ok_or_else(|| {
                    CoreError::new(
                        ReasonCode::Internal,
                        "incarnation founder artifact is not published",
                    )
                })?;
            let founder = shaula_template_manifest(&founder_text)?;
            if founder.platform != manifest.platform
                || founder.bindings_contract != manifest.bindings_contract
            {
                return Ok(Err(MutationError::IdentityConflict));
            }
        }
        if let Some(head) = head {
            if let Some(current) = plane
                .store
                .template_revision_get(key, head.desired_revision)
                .await?
            {
                let stored = plane
                    .store
                    .template_protected_bindings(key, head.desired_revision)
                    .await?
                    .map(|(json, _)| json)
                    .unwrap_or_default();
                if current.artifact_digest == payload.artifact_digest
                    && current.source_key == payload.source_key
                    && current.engine_ref == payload.engine_ref
                    && current.fleet_input_policy_json.as_deref() == Some(self.policy_json.as_str())
                    && stored == self.bindings_json
                {
                    return Ok(Ok(Plan::NoOp));
                }
            }
        }
        // Retained legacy revisions can be reasserted, but genuinely new
        // revisions must satisfy the current official-container policy.
        if let Err(error) = manifest.validate_new_container_profile().and_then(|()| {
            let bindings = payload.bindings.as_object().ok_or_else(|| {
                CoreError::new(
                    ReasonCode::TemplateInvalid,
                    "template bindings must be an object",
                )
            })?;
            manifest.runner_backend_for_bindings(bindings).map(|_| ())
        }) {
            return Ok(Err(unprocessable(error.code, error.summary)));
        }
        Ok(Ok(Plan::Revision {
            kind: "Publish",
            facts: manifest_yaml,
        }))
    }
    async fn commit(
        &self,
        plane: &ControlPlane,
        commit: Commit<'_>,
        manifest: Option<String>,
    ) -> Outcome<()> {
        let Some(manifest_yaml) = manifest else {
            let idempotency = commit
                .idempotency
                .as_ref()
                .map(|(key, ..)| (key.clone(), commit.canonical.to_string()));
            return plane
                .store
                .commit_template_noop(
                    commit.key,
                    commit.incarnation,
                    commit.revision(),
                    &commit.actor.name,
                    idempotency,
                    commit.now,
                )
                .await;
        };
        let payload = &self.publication.payload;
        let bindings_digest = shaula_core::template::BindingsDigest::from_keyed_material(
            &format!("{}/{}", commit.key, commit.revision()),
            &plane.bindings_server_key,
        )?
        .0;
        let mut facts = commit.facts(
            "template_profile",
            "profile.validate",
            format!(
                "{{\"key\":\"{}\",\"revision\":{}}}",
                commit.key,
                commit.revision()
            ),
        );
        // Preserve the legacy storage representation; splitting MutationFacts
        // and the extra tuple is deliberately not part of this refactor.
        facts.spec_json = manifest_yaml;
        facts.inputs_digest = payload.artifact_digest.clone();
        plane
            .store
            .commit_template_revision(
                facts,
                (
                    payload.engine_ref.clone(),
                    self.bindings_json.clone(),
                    bindings_digest,
                    self.policy_json.clone(),
                ),
                payload.source_key.clone(),
            )
            .await
    }
}
