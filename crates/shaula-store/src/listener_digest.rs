//! The pre-Jobs digest protected the lifecycle fields, but did not encode metadata.
use sha2::Digest;
use shaula_core::ports::PollMessage;

use crate::{StoreError, StoreResult};

pub(super) fn current(message: &PollMessage) -> StoreResult<String> {
    let payload = serde_json::to_vec(message)
        .map_err(|_| StoreError::Corrupt("listener message cannot be encoded".into()))?;
    Ok(hex::encode(sha2::Sha256::digest(payload)))
}

pub(super) fn legacy(message: &PollMessage) -> StoreResult<String> {
    let mut projected = message.clone();
    // Optional empty fields are omitted by serde, preserving the exact original
    // struct field ordering. Do not use a JSON map that could reorder hash input.
    for job in projected
        .job_available
        .iter_mut()
        .chain(&mut projected.job_assigned)
    {
        job.metadata = Default::default();
    }
    for job in &mut projected.job_started {
        job.metadata = Default::default();
    }
    for job in &mut projected.job_completed {
        job.metadata = Default::default();
        job.result = None;
    }
    current(&projected)
}

#[cfg(test)]
#[path = "listener_digest_tests.rs"]
mod tests;
