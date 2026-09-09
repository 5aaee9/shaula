//! Missing or malformed level sources must never become an authoritative zero.
use shaula_scaleset::wire::{RunnerScaleSetMessageResponse, RunnerScaleSetStatistic};

use super::{app_client, wire_mock_router, GitHubAccessPort};

#[tokio::test]
async fn session_statistics_are_required_and_nonnegative() {
    for statistics in [
        serde_json::Value::Null,
        serde_json::json!({}),
        serde_json::json!({"totalAssignedJobs": -1}),
        serde_json::json!({"totalAssignedJobs": 2, "totalBusyRunners": -1}),
    ] {
        let base = wire_mock_router::spawn_mock_github_with_statistics(statistics).await;
        let client = app_client(base);
        assert!(client.establish_session(42, "test").await.is_err());
    }
}

#[test]
fn message_statistics_are_required_and_nonnegative() {
    for statistics in [
        None,
        Some(RunnerScaleSetStatistic {
            total_assigned_jobs: -1,
            ..Default::default()
        }),
    ] {
        let response = RunnerScaleSetMessageResponse {
            message_id: 1,
            message_type: "RunnerScaleSetJobMessages".into(),
            body: "[]".into(),
            statistics,
        };
        assert!(shaula_scaleset::port::parse_job_messages(response).is_err());
    }
}

#[test]
fn statistics_without_assigned_demand_cannot_decode_as_zero() {
    assert!(serde_json::from_str::<RunnerScaleSetStatistic>(r#"{"totalBusyRunners":3}"#).is_err());
    assert!(serde_json::from_str::<RunnerScaleSetStatistic>(r#"{"totalAssignedJobs":0}"#).is_ok());
}
