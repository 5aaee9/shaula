use shaula_scaleset::{port::parse_job_messages, wire::RunnerScaleSetMessageResponse};

fn response(body: serde_json::Value) -> RunnerScaleSetMessageResponse {
    RunnerScaleSetMessageResponse {
        message_id: 1,
        message_type: "RunnerScaleSetJobMessages".into(),
        body: body.to_string(),
        statistics: Some(Default::default()),
    }
}

#[test]
fn approved_metadata_and_numeric_runner_identity_survive_the_wire_mapping(
) -> Result<(), Box<dyn std::error::Error>> {
    let message = parse_job_messages(response(serde_json::json!([{
        "messageType":"JobCompleted", "runnerRequestId":77,"jobId":"opaque-id","runnerId":42,"runnerName":"runner-42",
        "ownerName":"example","repositoryName":"repo","jobDisplayName":"test (linux)",
        "jobWorkflowRef":"example/repo/.github/workflows/ci.yml@refs/heads/main","workflowRunId":92,"eventName":"push",
        "queueTime":"2026-09-09T01:02:03.1234567Z","scaleSetAssignTime":"2026-09-09T01:02:04Z",
        "runnerAssignTime":"2026-09-09T01:02:05Z","finishTime":"2026-09-09T01:03:05Z","result":"succeeded",
        "acquireJobUrl":"https://secret.example.test/token","queueToken":"forbidden"
    }]))).map_err(|_| "job message did not decode")?;
    let completed = &message.job_completed[0];
    assert_eq!(completed.runner_id, 42);
    assert_eq!(completed.metadata.workflow_run_id, Some(92));
    assert_eq!(
        completed.metadata.job_display_name.as_deref(),
        Some("test (linux)")
    );
    assert_eq!(
        completed
            .metadata
            .queue_time
            .ok_or("missing timestamp")?
            .timestamp_subsec_nanos(),
        123_456_700
    );
    assert_eq!(completed.result.as_deref(), Some("succeeded"));
    let encoded = serde_json::to_string(&message)?;
    assert!(!encoded.contains("secret.example"));
    assert!(!encoded.contains("queueToken"));
    Ok(())
}

#[test]
fn missing_null_zero_and_oversize_display_fields_remain_unknown(
) -> Result<(), Box<dyn std::error::Error>> {
    let message = parse_job_messages(response(serde_json::json!([{
        "messageType":"JobAvailable","runnerRequestId":77,"repositoryName":null,"ownerName":"",
        "workflowRunId":0,"finishTime":"0001-01-01T00:00:00Z","jobDisplayName":"a".repeat(1025)
    }])))
    .map_err(|_| "job message did not decode")?;
    assert!(message.job_available[0].metadata.is_empty());
    assert!(message.job_available[0].job_id.is_empty());
    Ok(())
}

#[test]
fn incompatible_metadata_type_fails_the_batch_for_redelivery() {
    assert!(parse_job_messages(response(serde_json::json!([{
        "messageType":"JobAvailable","runnerRequestId":77,"workflowRunId":"not-a-number"
    }])))
    .is_err());
}
