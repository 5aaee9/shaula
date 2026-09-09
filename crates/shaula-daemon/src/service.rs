//! The control-plane application service: implements the Fleet and Profile
//! registry ports over core persistence ports. Mutations commit durable
//! desired state atomically and never call GitHub, Terraform or any
//! platform inside the admission path.

use std::sync::Arc;

use sha2::{Digest, Sha256};

use shaula_core::auth::AuthRevisionRef;
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::fleet::{FleetSpec, TemplateProfileRefDto};
use shaula_core::registry::{ControlPlaneStore, FleetHead, MutationAccepted, MutationError};

pub(crate) fn unprocessable(reason: ReasonCode, summary: impl Into<String>) -> MutationError {
    MutationError::Unprocessable {
        reason,
        summary: summary.into(),
    }
}

pub(crate) fn request_hash(parts: &[&[u8]]) -> String {
    // THE single hashing authority in core (R7-05): the length-prefixed
    // encoding keeps distinct part tuples — including ones that would
    // collide around an embedded NUL — on different digests.
    shaula_core::auth::request_hash_parts(parts)
}

/// The application service shared by HTTP and (future) CLI adapters.
pub struct ControlPlane {
    store: Arc<dyn ControlPlaneStore>,
    now: Arc<dyn shaula_core::ports::Clock>,
    bindings_server_key: Vec<u8>,
    ready: std::sync::atomic::AtomicBool,
    max_active_fleets: usize,
    /// THE configured engine binary — the authority for the attestation
    /// subject's `engine.binary_digest` (spec 0005 §5.1).
    engine_binary: std::path::PathBuf,
    /// Per-fleet effect gates (R5-02): a decommission's deletion commit
    /// holds the fleet's gate exclusively, serializing it against the
    /// admission claims the apply-intent sink hands to in-flight applies.
    effect_gates: Arc<crate::effect_gate::FleetEffectGates>,
}

impl ControlPlane {
    pub fn new(
        store: Arc<dyn ControlPlaneStore>,
        now: Arc<dyn shaula_core::ports::Clock>,
        bindings_server_key: Vec<u8>,
        max_active_fleets: usize,
        engine_binary: std::path::PathBuf,
    ) -> Self {
        Self {
            store,
            now,
            bindings_server_key,
            ready: std::sync::atomic::AtomicBool::new(false),
            max_active_fleets,
            engine_binary,
            effect_gates: Arc::new(crate::effect_gate::FleetEffectGates::new()),
        }
    }

    /// SHA-256 of the configured engine binary; a store that cannot
    /// prove its own engine fails every attestation (fail closed).
    fn engine_binary_digest(&self) -> CoreResult<String> {
        let bytes = std::fs::read(&self.engine_binary).map_err(|e| {
            CoreError::new(
                ReasonCode::TemplateInvalid,
                format!(
                    "engine binary {} unreadable: {e}",
                    self.engine_binary.display()
                ),
            )
        })?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        Ok(format!("sha256:{}", hex::encode(hasher.finalize())))
    }

    pub fn set_ready(&self, ready: bool) {
        self.ready.store(ready, std::sync::atomic::Ordering::SeqCst);
    }

    /// The fleet effect gates THIS control plane enforces. When the
    /// phase-3 supervisor wiring constructs the `LedgerApplyIntentSink`,
    /// it MUST inject THIS instance (never a fresh one): admission
    /// claims and decommission commits serialize per fleet only if both
    /// sides share the same gate instance (R5-02).
    pub fn effect_gates(&self) -> Arc<crate::effect_gate::FleetEffectGates> {
        Arc::clone(&self.effect_gates)
    }

    fn now_ms(&self) -> i64 {
        self.now.now_unix_ms()
    }

    fn new_id(&self) -> String {
        shaula_core::auth::new_attempt_id()
    }

    fn inputs_digest(&self, spec: &FleetSpec) -> String {
        let canonical = serde_json::to_vec(&spec.template_inputs).unwrap_or_default();
        format!("sha256:{}", hex::encode(Sha256::digest(&canonical)))
    }

    async fn resolve_template_ref(
        &self,
        reference: &TemplateProfileRefDto,
    ) -> CoreResult<(String, i64, String, String)> {
        let (key, requested) = match reference {
            TemplateProfileRefDto::BareKey(key) => (key.clone(), None),
            TemplateProfileRefDto::Exact { key, revision } => (
                key.clone(),
                Some(i64::try_from(*revision).unwrap_or(i64::MAX)),
            ),
        };
        let Some(profile) = self.store.template_profile_get(&key).await? else {
            return Err(CoreError::new(
                ReasonCode::TemplateNotFound,
                format!("template profile {key} not found"),
            ));
        };
        let Some(active) = profile.active_revision else {
            return Err(CoreError::new(
                ReasonCode::TemplateNotActive,
                format!("template profile {key} has no active revision"),
            ));
        };
        if let Some(requested) = requested {
            if requested != active {
                return Err(CoreError::new(
                    ReasonCode::TemplateNotActive,
                    format!(
                        "template revision {key}/{requested} is not the current active revision"
                    ),
                ));
            }
        }
        let revision = self
            .store
            .template_revision_get(&key, active)
            .await?
            .ok_or_else(|| CoreError::new(ReasonCode::Internal, "active revision row missing"))?;
        let attestation_id = profile.active_attestation_id.clone().ok_or_else(|| {
            CoreError::new(ReasonCode::Internal, "active template provenance missing")
        })?;
        Ok((key, active, revision.artifact_digest, attestation_id))
    }

    async fn resolve_auth_ref(&self, profile_key: &str) -> CoreResult<AuthRevisionRef> {
        let Some(profile) = self.store.auth_profile_get(profile_key).await? else {
            return Err(CoreError::new(
                ReasonCode::AuthProfileNotFound,
                format!("auth profile {profile_key} not found"),
            ));
        };
        let Some(active) = profile.active_revision else {
            return Err(CoreError::new(
                ReasonCode::AccessVerificationFailed,
                format!("auth profile {profile_key} has no active credential"),
            ));
        };
        Ok(AuthRevisionRef::new(
            shaula_core::auth::AuthProfileKey::new(profile_key)?,
            u64::try_from(active).unwrap_or(u64::MAX),
        ))
    }

    async fn assert_auth_target_allowed(
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
        let allowed = if revision.schema_version >= 2 {
            let policy = revision.target_policy()?.ok_or_else(|| {
                CoreError::new(ReasonCode::Internal, "active v2 auth policy missing")
            })?;
            policy.allows(&spec.github.target)
        } else {
            let allowlist: shaula_core::auth::TargetAllowlist =
                serde_json::from_str(&revision.allowlist_json).map_err(|e| {
                    CoreError::new(ReasonCode::Internal, format!("allowlist invalid: {e}"))
                })?;
            allowlist.allows(&spec.github.target)
        };
        if !allowed {
            return Err(CoreError::new(
                ReasonCode::AuthTargetDenied,
                "auth profile target allowlist does not cover the fleet target",
            ));
        }
        Ok(())
    }

    /// Canonical request hash for idempotency: method + resource + key +
    /// canonical body + precondition. The same formula must be used at
    /// lookup and at commit time.
    fn idempotency_hash(
        &self,
        kind: &str,
        key: &str,
        idem: &str,
        canonical_body: &str,
        precondition: &str,
    ) -> String {
        request_hash(&[
            kind.as_bytes(),
            key.as_bytes(),
            idem.as_bytes(),
            canonical_body.as_bytes(),
            precondition.as_bytes(),
        ])
    }

    /// Looks up a stored idempotent response for this exact request shape.
    async fn idempotency_replay(
        &self,
        kind: &str,
        key: &str,
        idempotency_key: &Option<String>,
        canonical_body: &str,
        precondition: &str,
    ) -> CoreResult<Result<Option<MutationAccepted>, MutationError>> {
        let Some(idem) = idempotency_key else {
            return Ok(Ok(None));
        };
        let hash = self.idempotency_hash(kind, key, idem, canonical_body, precondition);
        match self.store.idempotency_find(kind, key, idem, &hash).await? {
            shaula_core::registry::IdempotencyLookup::Miss => Ok(Ok(None)),
            shaula_core::registry::IdempotencyLookup::Conflict => {
                Ok(Err(MutationError::IdempotencyConflict))
            }
            shaula_core::registry::IdempotencyLookup::Replay(body) => {
                let accepted: MutationAccepted = serde_json::from_str(&body)
                    .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?;
                Ok(Ok(Some(accepted)))
            }
        }
    }
}

pub(crate) fn template_referenced(spec: &FleetSpec) -> String {
    match &spec.template_profile_ref {
        TemplateProfileRefDto::BareKey(key) => key.clone(),
        TemplateProfileRefDto::Exact { key, revision } => format!("{key}#{revision}"),
    }
}

#[path = "service_fleet_registry.rs"]
mod fleet_registry;

#[path = "service_fleet_ops.rs"]
mod fleet_ops;

#[path = "service_profile_registry.rs"]
mod profile_registry;

#[path = "service_profile_registry_put.rs"]
mod profile_registry_put;
pub(crate) use profile_registry::shaula_template_manifest;

#[path = "service_profile_attestation.rs"]
mod profile_attestation;

#[path = "service_auth.rs"]
mod auth;
#[path = "service_profile_conditions.rs"]
mod profile_conditions;
#[path = "service_profile_retirement.rs"]
mod profile_retirement;
#[path = "service_auth_view.rs"]
pub mod service_auth_view;

impl ControlPlane {
    /// No-op detection: identical canonical spec AND identical resolved
    /// template pin against the latest revision is a replayable no-op with
    /// the current ETag (spec 0002 §5.2/§5.3). Same-Profile auth promotion
    /// is deliberately NOT part of the comparison (§4.2: it must not
    /// modify the Fleet Revision or ETag); rotation propagates through the
    /// auth handoff retarget at promotion time instead.
    async fn detect_noop(
        &self,
        key: &str,
        head: Option<&FleetHead>,
        spec: &FleetSpec,
        template: Option<&(String, i64, String, String)>,
    ) -> CoreResult<Option<(String, i64)>> {
        let Some(head) = head else {
            return Ok(None);
        };
        let Some(previous) = self.store.fleet_revision_latest(key).await? else {
            return Ok(None);
        };
        let Ok(previous_spec) = serde_json::from_str::<FleetSpec>(&previous.spec_json) else {
            return Ok(None);
        };
        let same_spec = previous_spec == *spec;
        let same_template = match (&previous.template_profile_key, template) {
            (Some(k), Some(t)) => {
                k == &t.0
                    && previous.template_revision == Some(t.1)
                    && previous.template_artifact_digest.as_deref() == Some(t.2.as_str())
            }
            (None, None) => true,
            _ => false,
        };
        if same_spec && same_template {
            return Ok(Some((head.incarnation.clone(), head.desired_revision)));
        }
        Ok(None)
    }
}

#[path = "service_validation.rs"]
mod validation;

#[path = "service_input_contract.rs"]
mod input_contract;

#[path = "service_attestation.rs"]
mod attestation;

pub(crate) use validation::validate_inputs;

pub(crate) use attestation::verify_attestation_subject;
