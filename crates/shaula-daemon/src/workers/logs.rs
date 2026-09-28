use super::Workers;
use sha2::{Digest, Sha256};
use shaula_core::{
    state_backend::{StateError, StateResult},
    worker::{wire::*, ControlCapability},
};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[derive(Default)]
pub(super) struct LogState {
    invocations: HashSet<String>,
    begins: HashMap<Uuid, (String, String)>,
}

impl Workers {
    pub(super) async fn write_log(
        &self,
        generation: Uuid,
        token: &ControlCapability,
        request: LogRequest,
    ) -> StateResult<LogResponse> {
        self.authenticate(generation, token).await?;
        let session = self.session(generation).await?;
        let mut state = session.logs.lock().await;
        let sink = self.logs.as_ref().ok_or(StateError::Unavailable)?;
        match request.message {
            LogMessage::Begin(begin) => {
                if begin.generation_id != generation.to_string()
                    || !matches!(begin.operation.as_str(), "Create" | "Destroy")
                {
                    return Err(StateError::Unauthorized);
                }
                let digest = hex::encode(Sha256::digest(
                    serde_json::to_vec(&begin).map_err(|_| StateError::Invalid)?,
                ));
                if let Some((prior, id)) = state.begins.get(&request.request_id) {
                    if prior != &digest {
                        return Err(StateError::Conflict);
                    }
                    return Ok(LogResponse::Invocation(id.clone()));
                }
                if state.begins.len() >= 128 {
                    return Err(StateError::TooLarge);
                }
                let id = sink
                    .begin(begin)
                    .await
                    .map_err(|_| StateError::Unavailable)?;
                state.invocations.insert(id.clone());
                state
                    .begins
                    .insert(request.request_id, (digest, id.clone()));
                return Ok(LogResponse::Invocation(id));
            }
            LogMessage::Append(chunk) => {
                if !state.invocations.contains(&chunk.invocation_id) {
                    return Err(StateError::Unauthorized);
                }
                sink.append(chunk)
                    .await
                    .map_err(|_| StateError::Unavailable)?;
            }
            LogMessage::Command(command) => {
                if !state.invocations.contains(&command.invocation_id) {
                    return Err(StateError::Unauthorized);
                }
                sink.command(command)
                    .await
                    .map_err(|_| StateError::Unavailable)?;
            }
            LogMessage::Finish(finish) => {
                if !state.invocations.contains(&finish.invocation_id) {
                    return Err(StateError::Unauthorized);
                }
                sink.finish(finish)
                    .await
                    .map_err(|_| StateError::Unavailable)?;
            }
            LogMessage::Setup => {
                return Ok(LogResponse::Setup(
                    self.log_reader
                        .as_ref()
                        .ok_or(StateError::Unavailable)?
                        .setup_projection(&generation.to_string())
                        .await
                        .map_err(|_| StateError::Unavailable)?,
                ))
            }
        }
        Ok(LogResponse::Ack)
    }
}
