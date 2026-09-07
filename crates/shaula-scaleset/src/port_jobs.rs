//! Wire-to-core message mapping extracted from the port implementation.

use shaula_core::ports::{PollMessage, ScaleSetView, StatisticsSnapshot};

use crate::wire;

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
                label_type: l.label_type.clone(),
            })
            .collect(),
    }
}

pub(crate) fn stats_from_wire(value: wire::RunnerScaleSetStatistic) -> StatisticsSnapshot {
    StatisticsSnapshot {
        total_assigned_jobs: value.total_assigned_jobs,
        total_registered_runners: value.total_registered_runners,
        total_busy_runners: value.total_busy_runners,
        total_idle_runners: value.total_idle_runners,
    }
}

/// Private wire-shaped job messages (camelCase per the Actions Service
/// protocol); mapped onto core types before returning.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireJobMessage {
    runner_request_id: i64,
    #[serde(default)]
    job_id: String,
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
        statistics: response.statistics.map(stats_from_wire).unwrap_or_default(),
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
                });
            }
            "JobAssigned" => {
                let job = serde_json::from_value::<WireJobMessage>(entry).map_err(decode_failed)?;
                message.job_assigned.push(shaula_core::ports::JobMessage {
                    runner_request_id: job.runner_request_id,
                    job_id: job.job_id,
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
                    });
            }
            _ => {}
        }
    }
    Ok(message)
}
