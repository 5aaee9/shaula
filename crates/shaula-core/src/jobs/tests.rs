use super::*;

fn observation(kind: ObservationKind, episode: Option<i64>, runner: Option<i64>) -> JobObservation {
    JobObservation {
        id: format!("{kind:?}-{episode:?}"),
        kind,
        runner_request_id: 7,
        runner_id: runner,
        runner_name: runner.map(|r| format!("runner-{r}")),
        metadata: JobMetadata {
            scale_set_assign_time: episode.and_then(|t| chrono::DateTime::from_timestamp(t, 0)),
            ..Default::default()
        },
        reported_result: None,
        epoch: 1,
        message_id: 1,
        observed_at: 100,
        generation_id: runner.map(|r| format!("generation-{r}")),
        association_status: if runner.is_some() {
            AssociationStatus::Verified
        } else {
            AssociationStatus::Unverified
        },
    }
}

#[test]
fn late_start_cannot_erase_confirmed_completion() {
    let started = observation(ObservationKind::Started, Some(20), Some(42));
    let mut completed = observation(ObservationKind::Completed, Some(20), Some(42));
    completed.reported_result = Some("succeeded".into());
    for facts in [
        vec![started.clone(), completed.clone()],
        vec![completed, started],
    ] {
        let projection = project_observations(&facts);
        assert_eq!(projection.observed_status, ObservedStatus::Completed);
        assert_eq!(projection.reported_result.as_deref(), Some("succeeded"));
    }
}

#[test]
fn canceled_old_assignment_does_not_cancel_new_execution_even_with_same_request() {
    let mut canceled = observation(ObservationKind::Completed, Some(10), None);
    canceled.reported_result = Some("canceled".into());
    let started = observation(ObservationKind::Started, Some(20), Some(42));
    for facts in [
        vec![canceled.clone(), started.clone()],
        vec![started, canceled],
    ] {
        assert_eq!(
            project_observations(&facts).observed_status,
            ObservedStatus::Running
        );
    }
}

#[test]
fn request_id_and_arrival_order_do_not_resolve_an_unknown_cancellation_episode() {
    let mut canceled = observation(ObservationKind::Completed, None, None);
    canceled.reported_result = Some("canceled".into());
    let started = observation(ObservationKind::Started, None, Some(42));
    for facts in [
        vec![canceled.clone(), started.clone()],
        vec![started, canceled],
    ] {
        assert_eq!(
            project_observations(&facts).observed_status,
            ObservedStatus::Unknown
        );
    }
}

#[test]
fn new_source_assignment_can_follow_a_completed_execution() {
    let started = observation(ObservationKind::Started, Some(10), Some(42));
    let completed = observation(ObservationKind::Completed, Some(10), Some(42));
    let assigned = observation(ObservationKind::Assigned, Some(20), None);
    let facts = vec![assigned, completed, started];
    assert_eq!(
        project_observations(&facts).observed_status,
        ObservedStatus::Assigned
    );
}

#[test]
fn completed_without_started_reports_result_without_claiming_execution() {
    let mut completed = observation(ObservationKind::Completed, Some(10), Some(42));
    completed.reported_result = Some("failed".into());
    let result = project_observations(&[completed]);
    assert_eq!(result.observed_status, ObservedStatus::Unknown);
    assert_eq!(result.reported_result.as_deref(), Some("failed"));
}

#[test]
fn conflicting_protocol_identity_does_not_overwrite_run_identity() {
    let mut one = observation(ObservationKind::Available, Some(10), None);
    one.metadata.workflow_run_id = Some(1);
    let mut two = one.clone();
    two.metadata.workflow_run_id = Some(2);
    let result = project_observations(&[one, two]);
    assert_eq!(result.observed_status, ObservedStatus::Unknown);
    assert_eq!(result.association_status, AssociationStatus::Ambiguous);
    assert_eq!(result.metadata.workflow_run_id, None);
}

#[test]
fn only_valid_repository_metadata_produces_a_run_link() {
    let mut metadata = JobMetadata {
        owner_name: Some("example".into()),
        repository_name: Some("repo".into()),
        workflow_run_id: Some(9),
        ..Default::default()
    };
    assert_eq!(
        metadata.github_run_url().as_deref(),
        Some("https://github.com/example/repo/actions/runs/9")
    );
    metadata.repository_name = Some("../escape".into());
    assert_eq!(metadata.github_run_url(), None);
}

#[test]
fn empty_new_metadata_preserves_old_listener_message_digest_input() -> Result<(), serde_json::Error>
{
    let old = r#"{"runner_request_id":7,"job_id":"opaque"}"#;
    let parsed: crate::ports::JobMessage = serde_json::from_str(old)?;
    assert_eq!(serde_json::to_string(&parsed)?, old);
    Ok(())
}

#[test]
fn incomplete_timestamp_cannot_bridge_two_episodes_in_any_arrival_order() {
    for runner_timestamp in [false, true] {
        let mut facts = vec![
            observation(ObservationKind::Started, Some(10), Some(42)),
            observation(ObservationKind::Started, Some(20), Some(42)),
            observation(ObservationKind::Started, None, Some(42)),
            observation(ObservationKind::Completed, Some(10), Some(42)),
        ];
        facts[3].reported_result = Some("succeeded".into());
        if runner_timestamp {
            for fact in &mut facts {
                fact.metadata.runner_assign_time = fact.metadata.scale_set_assign_time.take();
            }
        }
        let orders = permutations(&facts);
        assert_eq!(orders.len(), 24);
        for order in orders {
            let result = project_observations(&order);
            assert_eq!(result.observed_status, ObservedStatus::Unknown, "{order:?}");
            assert_eq!(result.reported_result, None, "{order:?}");
        }
    }
}

#[test]
fn completion_missing_episode_time_cannot_finish_either_known_start() {
    let mut completed = observation(ObservationKind::Completed, None, Some(42));
    completed.reported_result = Some("succeeded".into());
    let facts = vec![
        observation(ObservationKind::Started, Some(10), Some(42)),
        observation(ObservationKind::Started, Some(20), Some(42)),
        completed,
    ];
    for order in permutations(&facts) {
        assert_eq!(
            project_observations(&order).observed_status,
            ObservedStatus::Unknown
        );
    }
}

fn permutations(facts: &[JobObservation]) -> Vec<Vec<JobObservation>> {
    if facts.is_empty() {
        return vec![Vec::new()];
    }
    let mut result = Vec::new();
    for (index, fact) in facts.iter().enumerate() {
        let mut remaining = facts.to_vec();
        remaining.remove(index);
        for mut order in permutations(&remaining) {
            order.insert(0, fact.clone());
            result.push(order);
        }
    }
    result
}
