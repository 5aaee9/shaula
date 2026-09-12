//! Forgejo composition never enters the GitHub handoff/session path.

use super::SupervisorWiring;
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::fleet::FleetProviderKind;
use shaula_core::registry::FleetRuntimeGuard;
use shaula_daemon::apply_intent::LedgerApplyIntentSink;
use shaula_daemon::forgejo_supervisor::{ForgejoPoolSupervisor, ForgejoPoolSupervisorDeps};
use std::sync::Arc;

impl SupervisorWiring {
    pub(super) async fn forgejo_supervisor_for(
        &self,
        key: &str,
        revision: i64,
    ) -> CoreResult<Option<Arc<ForgejoPoolSupervisor>>> {
        let Some(head) = self.store.fleet_get(key).await? else {
            return Ok(None);
        };
        let Some(latest) = self.store.fleet_revision_latest(key).await? else {
            return Ok(None);
        };
        // DELETE advances the head/fence, not the immutable specification.
        if head.tombstone
            || head.desired_revision != revision
            || (!head.deletion_marker && latest.revision != revision)
        {
            return Ok(None);
        }
        let spec = serde_json::from_str::<shaula_core::fleet::FleetSpec>(&latest.spec_json)
            .map_err(|_| CoreError::new(ReasonCode::SpecInvalid, "stored Fleet spec is invalid"))?;
        if spec.kind != FleetProviderKind::Forgejo {
            return Ok(None);
        }
        let Some(section) = spec.forgejo.as_ref() else {
            return Ok(None);
        };
        let (profile, mut auth_revision) = latest.auth_desired;
        // A revoked predecessor must not strand a deleting Fleet. Forgejo
        // profile targets (and User principal IDs) are immutable on rotation.
        if head.deletion_marker {
            if let Some(active) = self
                .store
                .auth_profile_get(&profile)
                .await?
                .and_then(|p| p.active_revision)
            {
                auth_revision = active;
            }
        }
        let Some(forgejo) = crate::wiring_credential::build_forgejo_client(
            &self.store,
            &profile,
            auth_revision,
            &section.target(),
        )
        .await?
        else {
            return Ok(None);
        };
        Ok(Some(Arc::new(ForgejoPoolSupervisor::new(
            key,
            section,
            spec.capacity.into(),
            ForgejoPoolSupervisorDeps {
                guard: FleetRuntimeGuard::from(&head),
                auth: (profile, auth_revision),
                gates: self.gates.clone(),
                store: self.store.clone(),
                lifecycle: self.lifecycle.clone(),
                forgejo,
                clock: self.clock.clone(),
                runtime: self.runtime.clone(),
                apply_intent_sink: Arc::new(LedgerApplyIntentSink {
                    store: self.lifecycle.clone(),
                    gates: self.gates.clone(),
                }),
                create_limit: self.limits.create.clone(),
                destroy_limit: self.limits.destroy.clone(),
                work_root: self.work_root.clone(),
                artifact_root: self.artifact_root.clone(),
                operation_timeout: self.operation_timeout,
                setup_info_issuer: self.setup_info_issuer.clone(),
            },
        )?)))
    }
}
