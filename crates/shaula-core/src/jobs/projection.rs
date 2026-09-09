//! Order-independent observation reduction; source evidence, never arrival order, selects an episode.

use super::{AssociationStatus, JobMetadata, JobObservation, ObservationKind, ObservedStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobProjection {
    pub metadata: JobMetadata,
    pub observed_status: ObservedStatus,
    pub reported_result: Option<String>,
    pub association_status: AssociationStatus,
}

pub fn project_observations(observations: &[JobObservation]) -> JobProjection {
    let metadata = merge_metadata(observations);
    let conflict = observations.iter().any(|left| {
        left.association_status == AssociationStatus::Ambiguous
            || observations
                .iter()
                .any(|right| left.metadata.identity_conflicts(&right.metadata))
    });
    let association_status = if conflict {
        AssociationStatus::Ambiguous
    } else if observations
        .iter()
        .any(|o| o.association_status == AssociationStatus::Verified)
    {
        AssociationStatus::Verified
    } else {
        AssociationStatus::Unverified
    };
    let (observed_status, reported_result) = if conflict {
        (ObservedStatus::Unknown, None)
    } else {
        execution_status(observations)
    };
    JobProjection {
        metadata,
        observed_status,
        reported_result,
        association_status,
    }
}

fn execution_status(observations: &[JobObservation]) -> (ObservedStatus, Option<String>) {
    let starts: Vec<_> = observations
        .iter()
        .filter(|o| {
            o.kind == ObservationKind::Started
                && o.association_status == AssociationStatus::Verified
        })
        .collect();
    if starts.is_empty() {
        return without_execution(observations);
    }
    let Some(current) = newest_start(&starts) else {
        return (ObservedStatus::Unknown, None);
    };
    let assignments: Vec<_> = observations
        .iter()
        .filter(|o| o.kind == ObservationKind::Assigned)
        .collect();
    if assignments
        .iter()
        .any(|assignment| strictly_older(current, assignment))
    {
        return if assignments.iter().any(|latest| {
            strictly_older(current, latest)
                && observations.iter().all(|other| {
                    other.kind == ObservationKind::Available
                        || strictly_older(other, latest)
                        || same_assignment(other, latest)
                })
        }) {
            (ObservedStatus::Assigned, None)
        } else {
            (ObservedStatus::Unknown, None)
        };
    }
    if assignments.iter().any(|assignment| {
        !same_assignment(current, assignment) && !strictly_older(assignment, current)
    }) {
        return (ObservedStatus::Unknown, None);
    }
    let mut completions = Vec::new();
    for observation in observations {
        if observation.kind != ObservationKind::Completed {
            continue;
        }
        if same_execution(current, observation) {
            completions.push(observation);
        } else if !strictly_older(observation, current) {
            // A canceled assignment without enough source identity can be a
            // different episode. Its arrival order cannot settle that question.
            return (ObservedStatus::Unknown, None);
        }
    }
    if completions.is_empty() {
        (ObservedStatus::Running, None)
    } else {
        let result = unique(completions.iter().map(|o| o.reported_result.as_ref()));
        if completions
            .iter()
            .any(|o| o.reported_result.is_some() && o.reported_result.as_ref() != result.as_ref())
        {
            (ObservedStatus::Unknown, None)
        } else {
            (ObservedStatus::Completed, result)
        }
    }
}

fn newest_start<'a>(starts: &[&'a JobObservation]) -> Option<&'a JobObservation> {
    starts.iter().copied().find(|candidate| {
        starts
            .iter()
            .all(|other| same_execution(candidate, other) || strictly_older(other, candidate))
    })
}

fn same_assignment(left: &JobObservation, right: &JobObservation) -> bool {
    if left.runner_request_id > 0 && right.runner_request_id > 0 {
        return left.runner_request_id == right.runner_request_id
            && left.metadata.scale_set_assign_time == right.metadata.scale_set_assign_time;
    }
    // The zero sentinel supplies no equality evidence. A direct assignment
    // can match a later request only through its source assignment time.
    left == right
        || (left.metadata.scale_set_assign_time.is_some()
            && left.metadata.scale_set_assign_time == right.metadata.scale_set_assign_time)
}

fn same_execution(left: &JobObservation, right: &JobObservation) -> bool {
    let request_evidence = if left.runner_request_id > 0 && right.runner_request_id > 0 {
        left.runner_request_id == right.runner_request_id
    } else {
        // Two unknown requests need independent source episode evidence. Do
        // not let an unknown request bridge distinct positive request IDs.
        left.runner_request_id == 0
            && right.runner_request_id == 0
            && (left == right
                || left.metadata.scale_set_assign_time.is_some()
                || left.metadata.runner_assign_time.is_some())
    };
    request_evidence
        && left.runner_id.is_some()
        && left.runner_id == right.runner_id
        && left.generation_id.is_some()
        && left.generation_id == right.generation_id
        // Absence is not a wildcard: a partially described event could belong
        // to either of two executions on this runner/request. Exact optional
        // timestamps make episode equality transitive and representative-free.
        && left.metadata.scale_set_assign_time == right.metadata.scale_set_assign_time
        && left.metadata.runner_assign_time == right.metadata.runner_assign_time
}

fn strictly_older(left: &JobObservation, right: &JobObservation) -> bool {
    match (
        left.metadata.scale_set_assign_time,
        right.metadata.scale_set_assign_time,
    ) {
        (Some(left), Some(right)) if left != right => left < right,
        _ => matches!(
            (left.metadata.runner_assign_time, right.metadata.runner_assign_time),
            (Some(left), Some(right)) if left < right
        ),
    }
}

fn without_execution(observations: &[JobObservation]) -> (ObservedStatus, Option<String>) {
    if observations
        .iter()
        .any(|o| o.kind == ObservationKind::Started)
    {
        return (ObservedStatus::Unknown, None);
    }
    let completed: Vec<_> = observations
        .iter()
        .filter(|o| o.kind == ObservationKind::Completed)
        .collect();
    if !completed.is_empty() {
        let withdrawn = completed
            .iter()
            .all(|o| o.runner_id.is_none() && o.reported_result.as_deref() == Some("canceled"));
        let newer_assignment = observations.iter().any(|o| {
            o.kind == ObservationKind::Assigned
                && completed.iter().all(|previous| strictly_older(previous, o))
        });
        return if newer_assignment {
            (ObservedStatus::Assigned, None)
        } else if withdrawn {
            (ObservedStatus::AssignmentWithdrawn, Some("canceled".into()))
        } else {
            (
                ObservedStatus::Unknown,
                unique(completed.iter().map(|o| o.reported_result.as_ref())),
            )
        };
    }
    if observations
        .iter()
        .any(|o| o.kind == ObservationKind::Assigned)
    {
        (ObservedStatus::Assigned, None)
    } else if observations
        .iter()
        .any(|o| o.kind == ObservationKind::Available)
    {
        (ObservedStatus::Queued, None)
    } else {
        (ObservedStatus::Unknown, None)
    }
}

fn unique<'a, T: Clone + Eq + 'a>(values: impl Iterator<Item = Option<&'a T>>) -> Option<T> {
    let mut values = values.flatten();
    let first = values.next()?;
    values.all(|value| value == first).then(|| first.clone())
}

fn merge_metadata(observations: &[JobObservation]) -> JobMetadata {
    macro_rules! field {
        ($name:ident) => {
            unique(observations.iter().map(|o| o.metadata.$name.as_ref()))
        };
    }
    JobMetadata {
        owner_name: field!(owner_name),
        repository_name: field!(repository_name),
        job_display_name: field!(job_display_name),
        job_workflow_ref: field!(job_workflow_ref),
        workflow_run_id: field!(workflow_run_id),
        event_name: field!(event_name),
        queue_time: field!(queue_time),
        scale_set_assign_time: field!(scale_set_assign_time),
        runner_assign_time: field!(runner_assign_time),
        finish_time: field!(finish_time),
    }
}
