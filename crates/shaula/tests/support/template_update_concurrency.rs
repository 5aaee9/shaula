use std::sync::{Arc, Barrier};

use shaula_core::ports::Clock;
use shaula_core::registry::{
    Actor, ControlPlaneStore, MutationAccepted, MutationError, ProfileRegistryPort, Scope,
    TemplateProfileUpdate,
};
use shaula_daemon::service::ControlPlane;

use super::support::{Fixture, TestResult};

// Publication reads now immediately before the commit. Hold both admitted
// requests here so both have observed an idempotency miss before either writes.
struct CommitBarrier(Barrier);

impl Clock for CommitBarrier {
    fn now_unix_ms(&self) -> i64 {
        self.0.wait();
        1_800_000_003_000
    }
}

type Outcome = Result<MutationAccepted, MutationError>;

async fn race(
    fixture: &Fixture,
    left: TemplateProfileUpdate,
    right: TemplateProfileUpdate,
) -> TestResult<(Outcome, Outcome)> {
    let service = Arc::new(ControlPlane::new(
        fixture.store.clone(),
        Arc::new(CommitBarrier(Barrier::new(2))),
        b"test-bindings-key".to_vec(),
        100,
        fixture.engine_binary.clone(),
    ));
    let head = fixture
        .store
        .template_profile_get("k8s-linux")
        .await?
        .ok_or("profile head missing")?;
    let publish = |payload| {
        let service = service.clone();
        let expected = (head.incarnation.clone(), head.desired_revision);
        tokio::spawn(async move {
            service
                .template_update(
                    &Actor {
                        name: "publisher".into(),
                        scopes: vec![Scope::TemplatePublish],
                    },
                    "k8s-linux",
                    payload,
                    Some(expected),
                    Some("simultaneous-update".into()),
                )
                .await
        })
    };
    let left = publish(left);
    let right = publish(right);
    Ok((left.await??, right.await??))
}

fn payload(fixture: &Fixture) -> TemplateProfileUpdate {
    TemplateProfileUpdate {
        source_key: None,
        artifact_digest: fixture.target_digest.clone(),
        engine_ref: "terraform".into(),
        fleet_input_policy: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn simultaneous_identical_updates_replay_the_winning_result() -> TestResult {
    let fixture = Fixture::new().await?;
    let (left, right) = race(&fixture, payload(&fixture), payload(&fixture)).await?;
    assert!(left.is_ok());
    assert_eq!(left, right);
    assert!(fixture
        .store
        .template_revision_get("k8s-linux", 3)
        .await?
        .is_none());
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn simultaneous_identical_noops_replay_without_a_unique_key_error() -> TestResult {
    let fixture = Fixture::new().await?;
    let mut update = payload(&fixture);
    update.artifact_digest = fixture.base_digest.clone();
    let (left, right) = race(&fixture, update.clone(), update).await?;
    assert!(left.as_ref().is_ok_and(|accepted| accepted.no_op));
    assert_eq!(left, right);
    assert!(fixture
        .store
        .template_revision_get("k8s-linux", 2)
        .await?
        .is_none());
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn simultaneous_changed_request_with_same_key_is_a_conflict() -> TestResult {
    let fixture = Fixture::new().await?;
    let mut other = payload(&fixture);
    other.fleet_input_policy = Some(serde_json::from_value(
        serde_json::json!({"size_class":["large"]}),
    )?);
    let (left, right) = race(&fixture, payload(&fixture), other).await?;
    let outcomes = [left, right];
    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == Err(MutationError::IdempotencyConflict))
            .count(),
        1
    );
    assert!(fixture
        .store
        .template_revision_get("k8s-linux", 3)
        .await?
        .is_none());
    Ok(())
}
