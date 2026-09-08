//! Profile Registry port implementation on the control-plane service.

use async_trait::async_trait;

use super::{unprocessable, ControlPlane};
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::registry::{
    Actor, AttestationPut, AttestationView, AuthProfilePut, AuthProfileView, AuthRevisionView,
    ChangeView, MutationAccepted, MutationError, ProfileRegistryPort, Scope, TemplateProfilePut,
    TemplateProfileView, TemplateRevisionView,
};

#[async_trait]
impl ProfileRegistryPort for ControlPlane {
    async fn template_input_contract_get(
        &self,
        actor: &Actor,
        key: &str,
        revision: i64,
    ) -> CoreResult<
        Result<
            shaula_core::registry::TemplateInputContract,
            shaula_core::registry::InputContractReadError,
        >,
    > {
        self.input_contract_get_impl(actor, key, revision).await
    }

    async fn template_put(
        &self,
        actor: &Actor,
        key: &str,
        payload: TemplateProfilePut,
        if_none_match: bool,
        if_match: Option<(String, i64)>,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>> {
        self.template_put_impl(
            actor,
            key,
            payload,
            if_none_match,
            if_match,
            idempotency_key,
        )
        .await
    }

    async fn template_get(
        &self,
        _actor: &Actor,
        key: &str,
    ) -> CoreResult<Result<TemplateProfileView, MutationError>> {
        let Some(profile) = self.store.template_profile_get(key).await? else {
            return Ok(Err(MutationError::NotFound));
        };
        let revision = self
            .store
            .template_revision_get(key, profile.desired_revision)
            .await?;
        Ok(Ok(TemplateProfileView {
            key: key.to_string(),
            incarnation: profile.incarnation.clone(),
            desired_revision: profile.desired_revision,
            active_revision: profile.active_revision,
            status: profile.status.clone(),
            platform: revision.as_ref().and_then(|r| r.platform.clone()),
            bindings_contract: revision.as_ref().and_then(|r| r.bindings_contract.clone()),
            bindings_present: revision
                .as_ref()
                .map(|r| r.bindings_present)
                .unwrap_or(false),
        }))
    }

    async fn template_list(&self, actor: &Actor) -> CoreResult<Vec<TemplateProfileView>> {
        let mut views = Vec::new();
        for key in self.store.template_profile_keys().await? {
            if let Ok(Ok(view)) = self.template_get(actor, &key).await {
                views.push(view);
            }
        }
        Ok(views)
    }

    async fn template_delete(
        &self,
        actor: &Actor,
        key: &str,
        idempotency_key: Option<String>,
        if_match: Option<(String, i64)>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>> {
        if !actor.has(Scope::TemplateRetire) {
            return Ok(Err(unprocessable(
                ReasonCode::SpecInvalid,
                "missing template.retire scope",
            )));
        }
        self.retire_profile(actor, key, true, if_match, idempotency_key)
            .await
    }

    async fn attestation_put(
        &self,
        actor: &Actor,
        key: &str,
        revision: i64,
        payload: AttestationPut,
    ) -> CoreResult<Result<String, MutationError>> {
        self.attestation_put_impl(actor, key, revision, payload)
            .await
    }

    async fn auth_put(
        &self,
        actor: &Actor,
        key: &str,
        payload: AuthProfilePut,
        if_none_match: bool,
        if_match: Option<(String, i64)>,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>> {
        self.auth_put_impl(
            actor,
            key,
            payload,
            if_none_match,
            if_match,
            idempotency_key,
        )
        .await
    }

    /// R10-05: read model of ONE immutable Auth Revision — credential
    /// bytes excluded by construction.
    async fn auth_revision_get(
        &self,
        actor: &Actor,
        key: &str,
        revision: i64,
    ) -> CoreResult<Result<AuthRevisionView, MutationError>> {
        if !actor.has(Scope::AuthRead) {
            return Ok(Err(unprocessable(
                ReasonCode::SpecInvalid,
                "missing auth.read scope",
            )));
        }
        let Some(row) = self.store.auth_revision_get(key, revision).await? else {
            return Ok(Err(MutationError::NotFound));
        };
        // Non-secret v2 metadata: the Target policy selectors and the
        // frozen Account Bindings of this exact revision.
        let target_policy = row.target_policy()?;
        let bindings = if row.schema_version >= 2 {
            self.store.auth_bindings_get(key, row.revision).await?
        } else {
            Vec::new()
        };
        Ok(Ok(AuthRevisionView {
            profile_key: row.profile_key,
            revision: row.revision,
            state: row.state,
            reason: row.reason,
            kind: row.kind,
            app_id: row.app_id,
            installation_id: row.installation_id,
            pat_principal: row.pat_principal,
            schema_version: row.schema_version,
            target_policy: target_policy.map(|p| p.selectors().to_vec()),
            bindings,
        }))
    }

    /// R10-05: read model of ONE immutable Template Revision.
    async fn template_revision_get(
        &self,
        actor: &Actor,
        key: &str,
        revision: i64,
    ) -> CoreResult<Result<TemplateRevisionView, MutationError>> {
        if !actor.has(Scope::TemplateRead) {
            return Ok(Err(unprocessable(
                ReasonCode::SpecInvalid,
                "missing template.read scope",
            )));
        }
        let Some(row) = self.store.template_revision_get(key, revision).await? else {
            return Ok(Err(MutationError::NotFound));
        };
        Ok(Ok(TemplateRevisionView {
            profile_key: row.profile_key,
            revision: row.revision,
            artifact_digest: row.artifact_digest,
            engine_ref: row.engine_ref,
            platform: row.platform,
            bindings_contract: row.bindings_contract,
            state: row.state,
            bindings_present: row.bindings_present,
        }))
    }

    /// R10-05: read model of ONE immutable attestation. The subject is
    /// the canonical typed serialization — secrets cannot appear here.
    async fn attestation_get(
        &self,
        actor: &Actor,
        key: &str,
        revision: i64,
        attestation_key: &str,
    ) -> CoreResult<Result<AttestationView, MutationError>> {
        if !actor.has(Scope::TemplateRead) {
            return Ok(Err(unprocessable(
                ReasonCode::SpecInvalid,
                "missing template.read scope",
            )));
        }
        let Some(row) = self
            .store
            .attestation_get(key, revision, attestation_key)
            .await?
        else {
            return Ok(Err(MutationError::NotFound));
        };
        Ok(Ok(AttestationView {
            profile_key: row.profile_key,
            revision: row.revision,
            subject: serde_json::from_str(&row.subject_json).unwrap_or(serde_json::Value::Null),
            result: row.result,
            suite: (
                row.suite_name.unwrap_or_default(),
                row.suite_version.unwrap_or_default(),
            ),
            completed_at: row.completed_at,
            subject_verified: row.subject_verified,
        }))
    }

    async fn auth_get(
        &self,
        _actor: &Actor,
        key: &str,
    ) -> CoreResult<Result<AuthProfileView, MutationError>> {
        self.auth_get_impl(_actor, key).await
    }
    async fn auth_list(&self, actor: &Actor) -> CoreResult<Vec<AuthProfileView>> {
        let mut views = Vec::new();
        for key in self.store.auth_profile_keys().await? {
            if let Ok(view) = self.auth_get(actor, &key).await? {
                views.push(view);
            }
        }
        views.sort_unstable_by(|left, right| left.key.cmp(&right.key));
        Ok(views)
    }

    async fn auth_delete(
        &self,
        actor: &Actor,
        key: &str,
        idempotency_key: Option<String>,
        if_match: Option<(String, i64)>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>> {
        if !actor.has(Scope::AuthRetire) {
            return Ok(Err(unprocessable(
                ReasonCode::SpecInvalid,
                "missing auth.retire scope",
            )));
        }
        self.retire_profile(actor, key, false, if_match, idempotency_key)
            .await
    }

    async fn profile_change_get(
        &self,
        _actor: &Actor,
        change_id: &str,
    ) -> CoreResult<Option<ChangeView>> {
        self.store.profile_change_get(change_id).await
    }
}

pub(crate) fn shaula_template_manifest(
    yaml: &str,
) -> CoreResult<shaula_core::template::ProfileManifest> {
    let manifest: shaula_core::template::ProfileManifest =
        serde_yaml::from_str(yaml).map_err(|e| {
            CoreError::new(
                ReasonCode::TemplateInvalid,
                format!("manifest invalid: {e}"),
            )
        })?;
    manifest.validate()?;
    Ok(manifest)
}
