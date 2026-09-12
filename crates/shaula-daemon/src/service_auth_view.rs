//! Revision-attributed, non-secret authentication read views.

use super::ControlPlane;
use shaula_core::error::CoreResult;
use shaula_core::registry::{
    Actor, AuthProfileView, AuthRevisionRow, AuthRevisionState, MutationError,
};

#[path = "service_auth_health.rs"]
mod health;

impl ControlPlane {
    pub(crate) async fn auth_get_impl(
        &self,
        _actor: &Actor,
        key: &str,
    ) -> CoreResult<Result<AuthProfileView, MutationError>> {
        let Some(profile) = self.store.auth_profile_get(key).await? else {
            return Ok(Err(MutationError::NotFound));
        };
        let active = self.store.auth_revision_active(key).await?;
        let desired = self
            .store
            .auth_revision_get(key, profile.desired_revision)
            .await?;
        let head = desired.as_ref().or(active.as_ref());
        let mut active_state = match &active {
            Some(row) => Some(self.auth_revision_state(row).await?),
            None => None,
        };
        let desired_state = match desired
            .as_ref()
            .filter(|row| Some(row.revision) != profile.active_revision)
        {
            Some(row) => Some(self.auth_revision_state(row).await?),
            None => None,
        };
        let dependents = self.store.auth_live_dependents(key).await?;
        let retained = dependents
            .iter()
            .flat_map(|dependent| {
                dependent
                    .retained_contexts
                    .iter()
                    .map(|context| (dependent.fleet_key.as_str(), context))
            })
            .collect::<Vec<_>>();
        // Read current Fleet conditions, never interpret persisted context as
        // fresh GitHub access proof. Failed candidates cannot taint active health.
        let mut routes = Vec::new();
        let mut seen_fleets = std::collections::BTreeSet::new();
        if active_state
            .as_ref()
            .is_some_and(|state| !state.bindings.is_empty())
        {
            for dependent in &dependents {
                if !seen_fleets.insert(&dependent.fleet_key) {
                    continue;
                }
                if let Some(context) = self
                    .store
                    .fleet_auth_context_get(&dependent.fleet_key)
                    .await?
                {
                    let handoff = self.store.handoff_get(&dependent.fleet_key).await?;
                    routes.push((context, handoff));
                }
            }
        }
        if let Some(state) = &mut active_state {
            let observations = self
                .store
                .auth_route_observations(key, state.revision, self.now_ms())
                .await?;
            state.binding_health = state
                .bindings
                .iter()
                .map(|binding| {
                    health::binding_health(
                        key,
                        state.revision,
                        binding,
                        &routes,
                        &observations,
                        &retained,
                        self.now_ms(),
                    )
                })
                .collect();
        }
        Ok(Ok(AuthProfileView {
            key: key.to_string(),
            incarnation: profile.incarnation,
            desired_revision: profile.desired_revision,
            active_revision: profile.active_revision,
            status: if head.is_some_and(|row| !supported(row))
                || active.as_ref().is_some_and(|row| !supported(row))
            {
                "Unsupported".into()
            } else {
                profile.status
            },
            kind: active
                .as_ref()
                .or(head)
                .and_then(|row| match row.kind.as_str() {
                    "pat" => Some(shaula_core::auth::AuthKind::Pat),
                    "github_app" => Some(shaula_core::auth::AuthKind::GithubApp),
                    "forgejo_token" => Some(shaula_core::auth::AuthKind::ForgejoToken),
                    _ => None,
                }),
            credential_present: active.is_some(),
            schema_version: head.map_or(1, |row| row.schema_version),
            app_id: head.and_then(|row| row.app_id.clone()),
            active: active_state,
            desired: desired_state,
            live_fleets: dependents
                .into_iter()
                .map(|dependent| shaula_core::registry::AuthLiveFleet {
                    fleet_key: dependent.fleet_key,
                    phase: dependent.fleet_phase,
                    target: serde_json::from_str(&dependent.target_json).ok(),
                    forgejo_target: serde_json::from_str(&dependent.target_json).ok(),
                })
                .collect(),
        }))
    }

    async fn auth_revision_state(&self, row: &AuthRevisionRow) -> CoreResult<AuthRevisionState> {
        let mut state = AuthRevisionState {
            revision: row.revision,
            state: row.state.clone(),
            reason: row.reason.clone(),
            binding_health: Vec::new(),
            schema_version: row.schema_version,
            app_id: None,
            target_policy: None,
            bindings: Vec::new(),
            forgejo: None,
        };
        if row.schema_version == 2 && row.kind == "github_app" {
            state.app_id = row.app_id.clone();
            state.target_policy = row.target_policy()?;
            state.bindings = self
                .store
                .auth_bindings_get(&row.profile_key, row.revision)
                .await?;
        } else if row.schema_version == 1 && row.kind == "forgejo_token" {
            let parse_error = |_| {
                shaula_core::error::CoreError::new(
                    shaula_core::error::ReasonCode::Internal,
                    "stored Forgejo authentication metadata is invalid",
                )
            };
            let target = serde_json::from_str(row.policy_json.as_deref().unwrap_or("null"))
                .map_err(parse_error)?;
            let validation = row
                .validation_snapshot_json
                .as_deref()
                .map(serde_json::from_str)
                .transpose()
                .map_err(parse_error)?;
            state.forgejo = Some(shaula_core::registry::ForgejoAuthState { target, validation });
        } else {
            state.state = "Unsupported".into();
            state.reason = Some("UnsupportedAuthenticationFormat".into());
        }
        Ok(state)
    }
}

fn supported(row: &AuthRevisionRow) -> bool {
    matches!(
        (row.kind.as_str(), row.schema_version),
        ("github_app", 2) | ("forgejo_token", 1)
    )
}
