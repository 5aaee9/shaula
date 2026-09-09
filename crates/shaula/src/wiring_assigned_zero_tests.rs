//! A real GitHub assignment can precede a positive runner request identity.

use super::*;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use serde_json::json;

#[tokio::test]
async fn assigned_zero_request_is_retained_and_acked_without_degrading_ready_fleet() {
    let (plane, mut wiring, listener, clock, _) = setup().await;
    ready(&plane, &mut wiring, &clock).await;
    *listener.message_response.lock().unwrap() = Some(json!({
        "messageId": 1,
        "messageType": "RunnerScaleSetJobMessages",
        "statistics": {"totalAssignedJobs": 1},
        "body": json!([{
            "messageType": "JobAssigned", "runnerRequestId": 0,
            "jobId": "job-assigned-zero", "ownerName": "5aaee9",
            "repositoryName": "proj", "workflowRunId": 123,
            "scaleSetAssignTime": "2026-09-09T20:10:00Z"
        }]).to_string()
    }));
    listener.emit_message.store(true, Ordering::SeqCst);
    tick(&mut wiring, &clock, NOW + 5_000).await;
    tick(&mut wiring, &clock, NOW + 6_000).await;
    let head = plane.control_plane.fleet_get(FLEET).await.unwrap().unwrap();
    assert_eq!(head.phase, "Ready", "{head:?}");
    assert_eq!(head.last_condition_reason, None);
    assert_eq!(
        plane.control_plane.demand_get(FLEET).await.unwrap(),
        Some(1)
    );
    assert_eq!(listener.acks.load(Ordering::SeqCst), 1);
    assert_eq!(listener.acquisitions.load(Ordering::SeqCst), 0);
    assert_eq!(listener.session_creates.load(Ordering::SeqCst), 1);
    assert_eq!(
        plane
            .control_plane
            .session_get(FLEET)
            .await
            .unwrap()
            .unwrap()
            .last_message_id,
        1
    );
    let db = sea_orm::Database::connect(format!("sqlite://{}?mode=ro", plane.db_path.display()))
        .await
        .unwrap();
    let row = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT j.status, o.runner_request_id FROM workflow_jobs j
         JOIN workflow_job_observations o ON o.job_record_id=j.id
         WHERE j.protocol_job_id='job-assigned-zero'"
                .to_owned(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<String>("", "status").unwrap(), "assigned");
    assert_eq!(row.try_get::<i64>("", "runner_request_id").unwrap(), 0);
    // Repeated delivery does not duplicate facts or repeat acquisition/ACK.
    listener.emit_message.store(true, Ordering::SeqCst);
    tick(&mut wiring, &clock, NOW + 7_000).await;
    assert_eq!(listener.acks.load(Ordering::SeqCst), 1);
    assert_eq!(listener.acquisitions.load(Ordering::SeqCst), 0);
    let row = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT COUNT(*) AS n FROM workflow_job_observations".to_owned(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<i64>("", "n").unwrap(), 1);
    wiring.tasks.shutdown().await;
}

#[tokio::test]
async fn direct_assignment_start_and_completion_with_zero_requests_keep_listener_healthy() {
    let (plane, mut wiring, listener, clock, _) = setup().await;
    ready(&plane, &mut wiring, &clock).await;
    for (index, kind) in ["JobAssigned", "JobStarted", "JobCompleted"]
        .iter()
        .enumerate()
    {
        let id = index as i64 + 1;
        let demand = if *kind == "JobCompleted" { 0 } else { 1 };
        *listener.message_response.lock().unwrap() = Some(json!({
            "messageId": id, "messageType": "RunnerScaleSetJobMessages",
            "statistics": {"totalAssignedJobs": demand},
            "body": json!([{
                "messageType": kind, "runnerRequestId": 0, "jobId": "direct-job",
                "runnerId": 91, "runnerName": "direct-runner",
                "scaleSetAssignTime": "2026-09-09T20:10:00Z",
                "runnerAssignTime": "2026-09-09T20:11:00Z",
                "result": "succeeded"
            }]).to_string()
        }));
        listener.emit_message.store(true, Ordering::SeqCst);
        tick(&mut wiring, &clock, NOW + 5_000 + id * 2_000).await;
        tick(&mut wiring, &clock, NOW + 6_000 + id * 2_000).await;
        let head = plane.control_plane.fleet_get(FLEET).await.unwrap().unwrap();
        assert_eq!(head.phase, "Ready", "{kind}: {head:?}");
        assert_eq!(head.last_condition_reason, None);
        assert_eq!(
            plane.control_plane.demand_get(FLEET).await.unwrap(),
            Some(demand)
        );
        assert_eq!(
            plane
                .control_plane
                .session_get(FLEET)
                .await
                .unwrap()
                .unwrap()
                .last_message_id,
            id
        );
        assert_eq!(listener.acks.load(Ordering::SeqCst), index + 1);
        assert_eq!(listener.acquisitions.load(Ordering::SeqCst), 0);
    }
    let db = sea_orm::Database::connect(format!("sqlite://{}?mode=ro", plane.db_path.display()))
        .await
        .unwrap();
    let row = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT COUNT(*) AS n FROM workflow_job_observations WHERE runner_request_id=0"
                .to_owned(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<i64>("", "n").unwrap(), 3);
    assert_eq!(listener.session_creates.load(Ordering::SeqCst), 1);
    wiring.tasks.shutdown().await;
}
