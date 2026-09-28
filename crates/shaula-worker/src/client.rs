use std::time::Duration;

use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use shaula_core::{
    state_backend::{StateError, StateResult, MAX_STATE_BYTES},
    worker::{wire::*, MAX_CONTROL_BYTES},
};
use uuid::Uuid;
use zeroize::Zeroizing;

pub struct WorkerClient {
    pub(crate) http: reqwest::Client,
    pub(crate) base: String,
    pub(crate) control: shaula_core::secret::SecretString,
    claim: shaula_core::state_backend::StateClaim,
}

impl WorkerClient {
    pub fn new(envelope: &LaunchEnvelope) -> StateResult<Self> {
        envelope.validate()?;
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(75))
            .build()
            .map_err(|_| StateError::Unavailable)?;
        Ok(Self {
            http,
            base: format!(
                "http://{}/internal/v1/generations/{}",
                envelope.address, envelope.claim.generation_id
            ),
            control: shaula_core::secret::SecretString::new(&envelope.control_capability),
            claim: envelope.claim.clone(),
        })
    }

    pub async fn call(&self, message: ControlMessage) -> StateResult<ControlResponse> {
        let request = ControlRequest {
            claim: self.claim.clone(),
            request_id: Uuid::new_v4(),
            message,
        };
        let bytes = Zeroizing::new(serde_json::to_vec(&request).map_err(|_| StateError::Invalid)?);
        if bytes.len() > MAX_CONTROL_BYTES {
            return Err(StateError::TooLarge);
        }
        // Same request identity and exact bytes on every transport retry. A
        // successful response lost in transit must never repeat a Create gate.
        for attempt in 0..3 {
            let response = self
                .http
                .post(format!("{}/control", self.base))
                .bearer_auth(self.control.expose())
                .header("Content-Type", "application/json")
                .body(bytes.to_vec())
                .send()
                .await;
            if let Ok(response) = response {
                let status = response.status();
                if status.is_success() {
                    let bytes = bounded(response, MAX_CONTROL_BYTES).await?;
                    return serde_json::from_slice(&bytes).map_err(|_| StateError::Invalid);
                }
                if !status.is_server_error() {
                    return Err(if status.as_u16() == 401 {
                        StateError::Unauthorized
                    } else {
                        StateError::Conflict
                    });
                }
            }
            if attempt < 2 {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
        Err(StateError::Unavailable)
    }

    pub async fn ack(&self, message: ControlMessage) -> StateResult<()> {
        match self.call(message).await? {
            ControlResponse::Ack => Ok(()),
            _ => Err(StateError::Invalid),
        }
    }

    pub async fn material<T: DeserializeOwned>(&self, reference: &MaterialRef) -> StateResult<T> {
        let response = self
            .http
            .get(format!("{}/materials/{}", self.base, reference.id))
            .bearer_auth(self.control.expose())
            .send()
            .await
            .map_err(|_| StateError::Unavailable)?;
        if !response.status().is_success() {
            return Err(StateError::Unauthorized);
        }
        let bytes = Zeroizing::new(bounded(response, MAX_STATE_BYTES).await?);
        if format!("sha256:{}", hex::encode(Sha256::digest(&bytes))) != reference.digest {
            return Err(StateError::Conflict);
        }
        serde_json::from_slice(&bytes).map_err(|_| StateError::Invalid)
    }
}

pub(crate) async fn bounded(mut response: reqwest::Response, max: usize) -> StateResult<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > max as u64)
    {
        return Err(StateError::TooLarge);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| StateError::Unavailable)?
    {
        if bytes.len().saturating_add(chunk.len()) > max {
            return Err(StateError::TooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
