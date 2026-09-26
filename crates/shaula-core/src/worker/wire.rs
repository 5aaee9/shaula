//! Private wire envelopes. Serialize only into a protected handoff/control
//! channel; never expose these as management response DTOs or Debug output.
use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{CompletionReceipt, ControlCapability, PROTOCOL_VERSION};
use crate::{
    ports::{
        ApplyIntentSink, DestroyClassification, OriginalStateIdentity, PlanProvenance,
        TemplateCreateRequest, TemplateCreateResult, TemplateDestroyRequest, TemplateOutcomeError,
    },
    secret::SecretString,
    state_backend::{StateClaim, StateError, StateResult},
    template::{BindingsDigest, ManagedResourceRole, ShaulaInputEnvelope},
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchEnvelope {
    pub protocol: u32,
    pub claim: StateClaim,
    pub address: SocketAddr,
    pub control_capability: String,
    pub state_capability: String,
    pub executable_digest: String,
    pub engine: PathBuf,
    pub work_root: PathBuf,
    pub artifact_root: PathBuf,
    pub workspace: PathBuf,
    pub artifact: PathBuf,
    pub artifact_digest: String,
    pub operation_timeout: Duration,
    pub cleanup_only: bool,
}

impl LaunchEnvelope {
    pub fn validate(&self) -> StateResult<()> {
        if self.protocol != PROTOCOL_VERSION
            || self.claim.worker_epoch <= 0
            || !self.address.ip().is_loopback()
            || self.address.port() == 0
            || !self.engine.is_absolute()
            || !self.workspace.is_absolute()
            || !self.artifact.is_absolute()
            || self.workspace.file_name().and_then(|v| v.to_str())
                != Some(&self.claim.generation_id.to_string())
            || self.operation_timeout.is_zero()
            || self.operation_timeout > super::MAX_OPERATION_TIMEOUT
        {
            return Err(StateError::Invalid);
        }
        ControlCapability::parse(SecretString::new(&self.control_capability))?;
        crate::state_backend::StateCapability::parse(SecretString::new(&self.state_capability))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialRef {
    pub id: Uuid,
    pub digest: String,
}

/// Material is fetched separately from the small control body. Its digest is
/// bound into the offered directive; a retry cannot substitute different input.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Directive {
    Wait,
    Create(MaterialRef),
    Destroy(MaterialRef),
    Complete(CompletionReceipt),
    Quiesce,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlRequest {
    pub claim: StateClaim,
    pub request_id: Uuid,
    pub message: ControlMessage,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum ControlMessage {
    Handshake {
        protocol: u32,
    },
    Poll,
    Prepared(Result<(), TemplateOutcomeError>),
    CreateFinished {
        material_id: Uuid,
        result: Result<TemplateCreateResult, TemplateOutcomeError>,
    },
    DestroyFinished {
        material_id: Uuid,
        result: Result<DestroyClassification, TemplateOutcomeError>,
    },
    ApplyStarting(PlanProvenance),
    Spawned(PlanProvenance),
    CommandEnded(PlanProvenance),
    BootstrapStarting(PlanProvenance),
    BootstrapEnded(PlanProvenance),
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum ControlResponse {
    Ack,
    Desired(Directive),
}

#[async_trait::async_trait]
pub trait WorkerControl: Send + Sync {
    /// Authenticate before body decoding; calls recheck the current Claim.
    async fn authenticate(
        &self,
        generation: Uuid,
        capability: &ControlCapability,
    ) -> StateResult<()>;
    async fn call(
        &self,
        capability: &ControlCapability,
        request: ControlRequest,
    ) -> StateResult<ControlResponse>;
    async fn material(
        &self,
        generation: Uuid,
        capability: &ControlCapability,
        id: Uuid,
    ) -> StateResult<Vec<u8>>;
    async fn logs(
        &self,
        _generation: Uuid,
        _capability: &ControlCapability,
        _request: LogRequest,
    ) -> StateResult<LogResponse> {
        Err(StateError::Unavailable)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogRequest {
    pub request_id: Uuid,
    pub message: LogMessage,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum LogMessage {
    Begin(crate::operation_log::BeginInvocation),
    Append(crate::operation_log::AppendLog),
    Command(crate::operation_log::LogCommand),
    Finish(crate::operation_log::FinishInvocation),
    Setup,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum LogResponse {
    Ack,
    Invocation(String),
    Setup(crate::operation_log::SetupProjection),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateMaterial {
    pub input: ShaulaInputEnvelope,
    pub expected_bindings_digest: BindingsDigest,
    pub managed_shape: Vec<ManagedResourceRole>,
    pub environment: Vec<(String, String)>,
    pub forgejo: Option<ForgejoMaterial>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForgejoMaterial {
    instance_url: String,
    uuid: String,
    token: String,
    labels: Vec<String>,
}

impl CreateMaterial {
    pub fn from_request(request: &TemplateCreateRequest) -> Self {
        Self {
            input: request.input.clone(),
            expected_bindings_digest: request.expected_bindings_digest.clone(),
            managed_shape: request.managed_shape.clone(),
            environment: request.environment.clone(),
            forgejo: request
                .forgejo_bootstrap
                .as_ref()
                .map(|material| ForgejoMaterial {
                    instance_url: material.instance_url.clone(),
                    uuid: material.uuid.clone(),
                    token: material.token().to_owned(),
                    labels: material.labels.clone(),
                }),
        }
    }

    pub fn into_request(
        self,
        launch: &LaunchEnvelope,
        sink: Arc<dyn ApplyIntentSink>,
    ) -> StateResult<TemplateCreateRequest> {
        if self.input.generation.id != launch.claim.generation_id.to_string() {
            return Err(StateError::Invalid);
        }
        let forgejo_bootstrap = self
            .forgejo
            .map(|material| {
                crate::ports::forgejo::ForgejoBootstrapMaterial::new(
                    material.instance_url,
                    material.uuid,
                    SecretString::new(material.token),
                    material.labels,
                )
                .map_err(|_| StateError::Invalid)
            })
            .transpose()?;
        Ok(TemplateCreateRequest {
            workspace_path: launch.workspace.clone(),
            artifact_dir: launch.artifact.clone(),
            pinned_artifact_digest: launch.artifact_digest.clone(),
            input: self.input,
            expected_bindings_digest: self.expected_bindings_digest,
            managed_shape: self.managed_shape,
            environment: self.environment,
            timeout: launch.operation_timeout,
            apply_intent_sink: Some(sink),
            forgejo_bootstrap,
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DestroyMaterial {
    pub protected_input: Option<String>,
    pub expected_bindings_digest: BindingsDigest,
    pub managed_shape: Vec<ManagedResourceRole>,
    pub environment: Vec<(String, String)>,
    pub original_provenance: PlanProvenance,
    pub original_state: OriginalStateIdentity,
}

impl DestroyMaterial {
    pub fn from_request(request: &TemplateDestroyRequest) -> Self {
        Self {
            protected_input: None,
            expected_bindings_digest: request.expected_bindings_digest.clone(),
            managed_shape: request.managed_shape.clone(),
            environment: request.environment.clone(),
            original_provenance: request.original_provenance.clone(),
            original_state: request.original_state.clone(),
        }
    }

    pub fn into_request(
        self,
        launch: &LaunchEnvelope,
        sink: Arc<dyn ApplyIntentSink>,
    ) -> StateResult<TemplateDestroyRequest> {
        if self.original_provenance.generation_id != launch.claim.generation_id.to_string() {
            return Err(StateError::Invalid);
        }
        Ok(TemplateDestroyRequest {
            workspace_path: launch.workspace.clone(),
            artifact_dir: launch.artifact.clone(),
            pinned_artifact_digest: launch.artifact_digest.clone(),
            generation_id: launch.claim.generation_id.to_string(),
            expected_bindings_digest: self.expected_bindings_digest,
            managed_shape: self.managed_shape,
            environment: self.environment,
            timeout: launch.operation_timeout,
            apply_intent_sink: Some(sink),
            original_provenance: self.original_provenance,
            original_state: self.original_state,
        })
    }
}
