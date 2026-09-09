//! Only the approved Scale Set metadata crosses the wire boundary.
use chrono::{DateTime, Datelike, Utc};
use shaula_core::jobs::JobMetadata;

#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WireJobMetadata {
    owner_name: Option<String>,
    repository_name: Option<String>,
    job_display_name: Option<String>,
    job_workflow_ref: Option<String>,
    workflow_run_id: Option<i64>,
    event_name: Option<String>,
    queue_time: Option<DateTime<Utc>>,
    scale_set_assign_time: Option<DateTime<Utc>>,
    runner_assign_time: Option<DateTime<Utc>>,
    finish_time: Option<DateTime<Utc>>,
}

impl From<WireJobMetadata> for JobMetadata {
    fn from(value: WireJobMetadata) -> Self {
        Self {
            owner_name: bounded_text(value.owner_name, 256),
            repository_name: bounded_text(value.repository_name, 256),
            job_display_name: bounded_text(value.job_display_name, 1024),
            job_workflow_ref: bounded_text(value.job_workflow_ref, 2048),
            workflow_run_id: value.workflow_run_id.filter(|id| *id > 0),
            event_name: bounded_text(value.event_name, 128),
            queue_time: meaningful_time(value.queue_time),
            scale_set_assign_time: meaningful_time(value.scale_set_assign_time),
            runner_assign_time: meaningful_time(value.runner_assign_time),
            finish_time: meaningful_time(value.finish_time),
        }
    }
}

pub(super) fn bounded_text(value: Option<String>, maximum: usize) -> Option<String> {
    value.filter(|text| !text.trim().is_empty() && text.len() <= maximum)
}

fn meaningful_time(value: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    value.filter(|time| time.year() > 1)
}
