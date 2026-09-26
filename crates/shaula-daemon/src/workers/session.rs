use sha2::{Digest, Sha256};
use shaula_core::{
    ports::{
        ApplyClaim, ApplyIntentSink, DestroyClassification, TemplateCreateResult,
        TemplateOutcomeError,
    },
    state_backend::{StateError, StateResult, MAX_STATE_BYTES},
    worker::{
        wire::{Directive, MaterialRef},
        ControlAccess, ProcessIdentity,
    },
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{oneshot, Mutex};
use uuid::Uuid;

pub(super) struct Session {
    pub access: ControlAccess,
    pub workspace: PathBuf,
    pub artifact: PathBuf,
    pub digest: String,
    pub identity: Mutex<Option<ProcessIdentity>>,
    pub state: Mutex<State>,
    pub logs: Mutex<super::logs::LogState>,
    pub permit: Mutex<Option<tokio::sync::OwnedSemaphorePermit>>,
    pub completed: Mutex<Option<tokio::time::Instant>>,
    pub cleanup_attempts: std::sync::atomic::AtomicU8,
    pub fenced: std::sync::atomic::AtomicBool,
}

impl Session {
    pub fn check_material(
        &self,
        workspace: &Path,
        artifact: &Path,
        digest: &str,
    ) -> StateResult<()> {
        if self.workspace != workspace || self.artifact != artifact || self.digest != digest {
            return Err(StateError::Conflict);
        }
        Ok(())
    }
}

pub(super) struct State {
    pub prepared: Option<oneshot::Sender<Result<(), TemplateOutcomeError>>>,
    pub prepared_ok: bool,
    pub created: bool,
    pub created_result: Option<oneshot::Sender<Result<TemplateCreateResult, TemplateOutcomeError>>>,
    pub destroyed_result:
        Option<oneshot::Sender<Result<DestroyClassification, TemplateOutcomeError>>>,
    pub desired: Option<Directive>,
    pub material: Option<(Uuid, Vec<u8>)>,
    pub sink: Option<Arc<dyn ApplyIntentSink>>,
    pub guard: Option<ApplyClaim>,
    pub bootstrap_guard: Option<ApplyClaim>,
    pub replay: HashMap<Uuid, (String, Vec<u8>)>,
}

impl State {
    pub fn new(prepared: oneshot::Sender<Result<(), TemplateOutcomeError>>) -> Self {
        Self {
            prepared: Some(prepared),
            prepared_ok: false,
            created: false,
            created_result: None,
            destroyed_result: None,
            desired: None,
            material: None,
            sink: None,
            guard: None,
            bootstrap_guard: None,
            replay: HashMap::new(),
        }
    }

    pub fn material(&mut self, value: &impl serde::Serialize) -> StateResult<MaterialRef> {
        let bytes = serde_json::to_vec(value).map_err(|_| StateError::Invalid)?;
        if bytes.len() > MAX_STATE_BYTES {
            return Err(StateError::TooLarge);
        }
        let id = Uuid::new_v4();
        let digest = format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
        self.material = Some((id, bytes));
        Ok(MaterialRef { id, digest })
    }
}
