//! Wire-to-core message mapping extracted from the port implementation.

use shaula_core::ports::{PollMessage, ScaleSetView, StatisticsSnapshot};

use crate::wire;

#[path = "port_job_metadata.rs"]
mod metadata;
use metadata::WireJobMetadata;

pub(crate) fn scale_set_view(value: &wire::RunnerScaleSet) -> ScaleSetView {
    ScaleSetView {
        id: value.id.unwrap_or_default(),
        name: value.name.clone().unwrap_or_default(),
        runner_group_id: value.runner_group_id.unwrap_or_default(),
        runner_group_name: value.runner_group_name.clone().unwrap_or_default(),
        labels: value
            .labels
            .iter()
            .map(|l| shaula_core::github::Label {
                name: l.name.clone(),
                label_type: l.label_type.as_str().to_string(),
            })
            .collect(),
    }
}

pub(crate) fn stats_from_wire(
    value: Option<wire::RunnerScaleSetStatistic>,
) -> Result<StatisticsSnapshot, shaula_core::ports::AccessFailure> {
    let value = value.ok_or_else(|| shaula_core::ports::AccessFailure::Unavailable {
        summary: "message statistics missing".into(),
    })?;
    if [
        value.total_assigned_jobs,
        value.total_registered_runners,
        value.total_busy_runners,
        value.total_idle_runners,
        value.total_available_jobs,
        value.total_acquired_jobs,
        value.total_running_jobs,
    ]
    .into_iter()
    .any(|count| count < 0)
    {
        return Err(shaula_core::ports::AccessFailure::Unavailable {
            summary: "message statistics contain a negative count".into(),
        });
    }
    Ok(StatisticsSnapshot {
        total_assigned_jobs: value.total_assigned_jobs,
        total_registered_runners: value.total_registered_runners,
        total_busy_runners: value.total_busy_runners,
        total_idle_runners: value.total_idle_runners,
    })
}

/// Private wire-shaped job messages (camelCase per the Actions Service
/// protocol); mapped onto core types before returning.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireJobMessage {
    runner_request_id: i64,
    #[serde(default)]
    job_id: String,
    #[serde(flatten)]
    metadata: WireJobMetadata,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireJobStarted {
    runner_request_id: i64,
    #[serde(default)]
    job_id: String,
    #[serde(default)]
    runner_id: i64,
    #[serde(default)]
    runner_name: String,
    #[serde(flatten)]
    metadata: WireJobMetadata,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireJobCompleted {
    runner_request_id: i64,
    #[serde(default)]
    job_id: String,
    #[serde(default)]
    runner_id: i64,
    #[serde(default)]
    runner_name: String,
    #[serde(flatten)]
    metadata: WireJobMetadata,
    result: Option<String>,
}

/// A KNOWN job type that fails to decode fails the WHOLE batch: the
/// message is not ACKed and stays in the queue for redelivery - silent
/// dropping would permanently lose the observation (spec 0001 ingest
/// order). Unknown message types remain explicitly tolerated.
fn decode_failed(e: serde_json::Error) -> shaula_core::ports::AccessFailure {
    shaula_core::ports::AccessFailure::Unavailable {
        summary: format!("known job event failed to decode: {e}"),
    }
}

pub fn parse_job_messages(
    response: wire::RunnerScaleSetMessageResponse,
) -> Result<PollMessage, shaula_core::ports::AccessFailure> {
    let batch: Vec<serde_json::Value> = if response.body.is_empty() {
        Vec::new()
    } else {
        serde_json::from_str(&response.body).map_err(|e| {
            shaula_core::ports::AccessFailure::Unavailable {
                summary: format!("message batch invalid: {e}"),
            }
        })?
    };
    let mut message = PollMessage {
        message_id: response.message_id,
        statistics: stats_from_wire(response.statistics)?,
        job_available: Vec::new(),
        job_assigned: Vec::new(),
        job_started: Vec::new(),
        job_completed: Vec::new(),
    };
    for entry in batch {
        let message_type = entry
            .get("messageType")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        match message_type.as_str() {
            "JobAvailable" => {
                let job = serde_json::from_value::<WireJobMessage>(entry).map_err(decode_failed)?;
                message.job_available.push(shaula_core::ports::JobMessage {
                    runner_request_id: job.runner_request_id,
                    job_id: job.job_id,
                    metadata: job.metadata.into(),
                });
            }
            "JobAssigned" => {
                let job = serde_json::from_value::<WireJobMessage>(entry).map_err(decode_failed)?;
                message.job_assigned.push(shaula_core::ports::JobMessage {
                    runner_request_id: job.runner_request_id,
                    job_id: job.job_id,
                    metadata: job.metadata.into(),
                });
            }
            "JobStarted" => {
                let job = serde_json::from_value::<WireJobStarted>(entry).map_err(decode_failed)?;
                message
                    .job_started
                    .push(shaula_core::ports::JobStartedMessage {
                        runner_request_id: job.runner_request_id,
                        job_id: job.job_id,
                        runner_id: job.runner_id,
                        runner_name: job.runner_name,
                        metadata: job.metadata.into(),
                    });
            }
            "JobCompleted" => {
                let job =
                    serde_json::from_value::<WireJobCompleted>(entry).map_err(decode_failed)?;
                message
                    .job_completed
                    .push(shaula_core::ports::JobCompletedMessage {
                        runner_request_id: job.runner_request_id,
                        job_id: job.job_id,
                        runner_id: job.runner_id,
                        runner_name: job.runner_name,
                        metadata: job.metadata.into(),
                        result: metadata::bounded_text(job.result, 128),
                    });
            }
            _ => {}
        }
    }
    Ok(message)
}
