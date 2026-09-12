//! Provider-specific admission of an exact active authentication target.

use super::ControlPlane;
use shaula_core::{
    error::{CoreError, CoreResult, ReasonCode},
    fleet::{FleetProviderKind, FleetSpec},
};

impl ControlPlane {
    pub(super) async fn assert_auth_target_allowed(
        &self,
        profile_key: &str,
        spec: &FleetSpec,
    ) -> CoreResult<()> {
        let Some(revision) = self.store.auth_revision_active(profile_key).await? else {
            return Err(CoreError::new(
                ReasonCode::AccessVerificationFailed,
                "auth credential missing",
            ));
        };
        match spec.kind {
            FleetProviderKind::Github => {
                if revision.schema_version != 2 || revision.kind != "github_app" {
                    return Err(CoreError::new(
                        ReasonCode::AuthTargetDenied,
                        "unsupported authentication revision; publish a GitHub App v2 profile",
                    ));
                }
                let policy = revision.target_policy()?.ok_or_else(|| {
                    CoreError::new(ReasonCode::Internal, "active auth policy missing")
                })?;
                if !policy.allows(&spec.github.target) {
                    return Err(CoreError::new(
                        ReasonCode::AuthTargetDenied,
                        "auth profile target policy does not cover the fleet target",
                    ));
                }
            }
            FleetProviderKind::Forgejo => {
                if revision.schema_version != 1 || revision.kind != "forgejo_token" {
                    return Err(CoreError::new(
                        ReasonCode::AuthTargetDenied,
                        "active authentication revision is not a Forgejo token",
                    ));
                }
                let probe = revision
                    .validation_snapshot_json
                    .as_deref()
                    .and_then(|json| {
                        serde_json::from_str::<shaula_core::ports::forgejo::ForgejoAuthProbe>(json)
                            .ok()
                    });
                if probe.as_ref().is_none_or(|probe| {
                    !shaula_core::forgejo::supports_server_version(&probe.server_version)
                }) {
                    return Err(CoreError::new(ReasonCode::AuthTargetDenied,
                        "Forgejo server version is unverified; publish and validate a new token revision"));
                }
                let section = spec.forgejo.as_ref().ok_or_else(|| {
                    CoreError::new(ReasonCode::SpecInvalid, "forgejo section is required")
                })?;
                let target = revision.policy_json.as_deref().and_then(|json| {
                    serde_json::from_str::<shaula_core::forgejo::ForgejoTarget>(json).ok()
                });
                if target.as_ref() != Some(&section.target()) {
                    return Err(CoreError::new(
                        ReasonCode::AuthTargetDenied,
                        "Forgejo auth profile target does not match the fleet scope",
                    ));
                }
            }
        }
        Ok(())
    }
}
