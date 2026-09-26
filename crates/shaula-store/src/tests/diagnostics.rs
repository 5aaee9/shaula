//! Real SQLite projection/fault tests. No external backend is queried by reads.
use super::runtime_sessions::established;
use sea_orm::ConnectionTrait;
use shaula_core::{
    diagnostics::*,
    registry::{Actor, FleetRuntimeGuard, Scope},
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
fn actor() -> Actor {
    Actor {
        name: "diagnostic-reader".into(),
        authentication: Default::default(),
        scopes: vec![Scope::FleetRead],
    }
}
async fn observer(store: &crate::Store) -> Result<Observer> {
    let head = store.fleet_get("fleet").await?.ok_or("fleet")?;
    let guard = Guard::fleet(
        "fleet",
        &FleetRuntimeGuard {
            incarnation: head.incarnation,
            desired_revision: head.desired_revision,
            mutation_fence: head.mutation_fence,
        },
    );
    Observer::register(
        Some(store.diagnostics.clone()),
        guard,
        Lane::Supervisor,
        QuestionId::ScaleUp,
    )
    .ok_or_else(|| "observer".into())
}
fn blocked(observer: &Observer, now: i64) -> Result<ObservationTicket> {
    let mut ticket = observer.begin(now).ok_or("ticket")?;
    ticket.observation.question.reason(
        Code::CapacityOccupancyLimit,
        StageId::Capacity,
        Effect::Blocking,
        EvidenceKind::DerivedCalculation,
    );
    Ok(ticket)
}
async fn read(store: &crate::Store, now: i64) -> Result<DiagnosticReportV1> {
    store
        .diagnostics(SubjectKind::Fleet, "fleet", &actor(), now)
        .await?
        .ok_or_else(|| "report".into())
}
async fn settled(store: &crate::Store, observation: &DecisionObservation) -> Result {
    for _ in 0..100 {
        let report = read(store, observation.observed_at).await?;
        if report.questions[0].basis.observation_id == observation.question.basis.observation_id {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    Err("snapshot writer did not settle".into())
}

#[tokio::test]
async fn repeated_reads_do_not_refresh_time_and_stale_blocker_is_not_primary() -> Result {
    let (store, _) = established().await?;
    let observer = observer(&store).await?;
    let ticket = blocked(&observer, 100)?;
    let observation = ticket.observation.clone();
    ticket.publish();
    settled(&store, &observation).await?;
    let fresh = read(&store, 200).await?;
    assert_eq!(fresh.questions[0].outcome, Outcome::Blocked);
    assert_eq!(
        fresh.questions[0].stages.last().map(|s| s.evaluation),
        Some(Evaluation::NotEvaluated)
    );
    let expired = read(&store, 30_101).await?;
    assert_eq!(expired.questions[0].outcome, Outcome::Unknown);
    assert_eq!(expired.questions[0].basis.freshness, Freshness::Stale);
    assert_eq!(expired.questions[0].primary_reason_id, None);
    assert_eq!(
        fresh.questions[0].basis.observed_at,
        expired.questions[0].basis.observed_at
    );
    assert_ne!(fresh.generated_at, expired.generated_at);
    Ok(())
}

#[tokio::test]
async fn older_sequence_and_retired_epoch_cannot_overwrite_new_work() -> Result {
    let (store, _) = established().await?;
    let old = observer(&store).await?;
    let first = blocked(&old, 100)?;
    let second = blocked(&old, 101)?;
    let observation = second.observation.clone();
    second.publish();
    first.publish();
    settled(&store, &observation).await?;
    assert_eq!(
        read(&store, 102).await?.questions[0].basis.observation_id,
        observation.question.basis.observation_id
    );
    let new = observer(&store).await?;
    let late = blocked(&old, 102);
    assert!(late.is_err());
    let new_ticket = blocked(&new, 103)?;
    let observation = new_ticket.observation.clone();
    new_ticket.publish();
    settled(&store, &observation).await?;
    Ok(())
}

#[tokio::test]
async fn conflicting_duplicate_and_future_clock_never_claim_fresh() -> Result {
    let (store, _) = established().await?;
    let observer = observer(&store).await?;
    let ticket = blocked(&observer, 100)?;
    let mut conflict = ticket.observation.clone();
    ticket.publish();
    settled(&store, &conflict).await?;
    conflict.question.outcome = Outcome::Satisfied;
    store.diagnostics.publish(conflict);
    assert_ne!(
        read(&store, 101).await?.questions[0].basis.freshness,
        Freshness::Fresh
    );
    let ticket = blocked(&observer, 500)?;
    let observation = ticket.observation.clone();
    ticket.publish();
    settled(&store, &observation).await?;
    assert_eq!(
        read(&store, 400).await?.questions[0].outcome,
        Outcome::Unknown
    );
    let rolled_back = blocked(&observer, 300)?;
    let observation = rolled_back.observation.clone();
    rolled_back.publish();
    settled(&store, &observation).await?;
    let report = read(&store, 600).await?;
    assert_eq!(report.questions[0].outcome, Outcome::Unknown);
    assert!(report.questions[0]
        .reasons
        .iter()
        .any(|r| r.code == Code::ObservationClockInvalid.as_str()));
    Ok(())
}

#[tokio::test]
async fn revision_change_and_projection_loss_preserve_authoritative_facts() -> Result {
    let (store, _) = established().await?;
    let observer = observer(&store).await?;
    let ticket = blocked(&observer, 100)?;
    let observation = ticket.observation.clone();
    ticket.publish();
    settled(&store, &observation).await?;
    store
        .connection()
        .execute_unprepared("UPDATE fleets SET mutation_fence=mutation_fence+1 WHERE key='fleet'")
        .await?;
    assert_eq!(
        read(&store, 101).await?.questions[0].outcome,
        Outcome::Unknown
    );
    store
        .connection()
        .execute_unprepared("DROP TABLE diagnostic_snapshots")
        .await?;
    assert!(store.fleet_get("fleet").await?.is_some());
    assert_eq!(read(&store, 102).await?.questions.len(), 3);
    store
        .connection()
        .execute_unprepared("ALTER TABLE fleets RENAME TO unavailable_fleets")
        .await?;
    assert!(read(&store, 103).await.is_err());
    Ok(())
}

#[tokio::test]
async fn continuous_reason_keeps_first_timestamp_and_a_gap_resets_it() -> Result {
    let (store, _) = established().await?;
    let observer = observer(&store).await?;
    for now in [100, 200, 30_300] {
        let ticket = blocked(&observer, now)?;
        let observation = ticket.observation.clone();
        ticket.publish();
        settled(&store, &observation).await?;
        let first = read(&store, now).await?.questions[0].reasons[0]
            .first_observed_at
            .clone();
        assert_eq!(first, timestamp(if now == 200 { 100 } else { now }));
    }
    Ok(())
}

#[tokio::test]
async fn diagnostic_overflow_failure_and_recovery_do_not_change_fleet() -> Result {
    let (store, _) = established().await?;
    let observer = observer(&store).await?;
    let ticket = blocked(&observer, 100)?;
    let observation = ticket.observation.clone();
    ticket.publish();
    settled(&store, &observation).await?;
    let before = store.fleet_get("fleet").await?.ok_or("fleet")?;
    let mut oversized = blocked(&observer, 101)?;
    oversized.observation.question.reasons[0].code = "x".repeat(65536);
    oversized.publish();
    assert_ne!(
        read(&store, 102).await?.questions[0].basis.freshness,
        Freshness::Fresh
    );
    store.connection().execute_unprepared("CREATE TRIGGER diagnostic_fault BEFORE INSERT ON diagnostic_snapshots BEGIN SELECT RAISE(FAIL,'injected'); END").await?;
    blocked(&observer, 103)?.publish();
    for _ in 0..100 {
        if store
            .diagnostics
            .write_failures
            .load(std::sync::atomic::Ordering::Relaxed)
            > 0
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        store
            .diagnostics
            .write_failures
            .load(std::sync::atomic::Ordering::Relaxed)
            > 0
    );
    assert_ne!(
        read(&store, 104).await?.questions[0].basis.freshness,
        Freshness::Fresh
    );
    store
        .connection()
        .execute_unprepared("DROP TRIGGER diagnostic_fault")
        .await?;
    let next = blocked(&observer, 105)?;
    let observation = next.observation.clone();
    next.publish();
    settled(&store, &observation).await?;
    assert_eq!(
        read(&store, 106).await?.questions[0].basis.freshness,
        Freshness::Fresh
    );
    let after = store.fleet_get("fleet").await?.ok_or("fleet")?;
    assert_eq!(before.desired_revision, after.desired_revision);
    assert_eq!(before.mutation_fence, after.mutation_fence);
    Ok(())
}

#[tokio::test]
async fn diagnostics_aggregates_one_hundred_thousand_rows_without_expanding_response() -> Result {
    let (store, _) = established().await?;
    store.connection().execute_unprepared("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<100000)
        INSERT INTO runner_generations(id,fleet_key,runner_name,generation_name,fleet_revision,template_profile_key,template_revision,template_artifact_digest,attestation_id,inputs_digest,state,workspace_path,created_at,updated_at)
        SELECT 'gen-'||x,'fleet','runner-'||x,'resource-'||x,1,'profile',1,'digest','attestation','inputs','Quarantined','private',1,1 FROM n").await?;
    let report = read(&store, 200).await?;
    assert_eq!(report.questions[1].outcome, Outcome::Blocked);
    assert!(serde_json::to_vec(&report)?.len() < 16_384);
    assert_eq!(report.related.len(), 0);
    Ok(())
}

#[tokio::test]
async fn diagnostics_gc_preserves_domain_and_restart_invalidates_runtime_evidence() -> Result {
    let (store, _) = established().await?;
    let observer = observer(&store).await?;
    let ticket = blocked(&observer, 100)?;
    let observation = ticket.observation.clone();
    ticket.publish();
    settled(&store, &observation).await?;
    let restarted_hub = crate::diagnostics::Hub::start(store.connection().clone());
    assert!(!restarted_hub.current(&observation));
    crate::diagnostics::collect(store.connection(), 7 * 86_400_000 + 101).await?;
    assert_eq!(
        read(&store, 102).await?.questions[0].basis.kind,
        BasisKind::Unavailable
    );
    assert!(store.fleet_get("fleet").await?.is_some());
    let next = blocked(&observer, 103)?;
    let observation = next.observation.clone();
    next.publish();
    settled(&store, &observation).await?;
    Ok(())
}

#[tokio::test]
async fn hard_lifetime_mode_survives_projection_loss_and_cleanup_checkpoints() -> Result {
    let (mut store, _) = established().await?;
    store.connection().execute_unprepared("INSERT INTO runner_generations
        (id,fleet_key,runner_name,generation_name,fleet_revision,template_profile_key,template_revision,template_artifact_digest,attestation_id,inputs_digest,state,workspace_path,created_at,updated_at,expiry_requested_at)
        VALUES('expired','fleet','runner','resource',1,'profile',1,'digest','attestation','inputs','DestroyPending','private',1,100,90)").await?;
    store
        .connection()
        .execute_unprepared(
            "INSERT INTO runner_operations
        (id,generation_id,kind,state,attempts,created_at,updated_at)
        VALUES('destroy','expired','Destroy','ApplyStarting',0,95,100)",
        )
        .await?;
    // Model restart with no process-local producer or retained projection.
    store.diagnostics = crate::diagnostics::Hub::start(store.connection().clone());
    store
        .connection()
        .execute_unprepared("DROP TABLE diagnostic_snapshots")
        .await?;
    for resource_gone in [false, true] {
        if resource_gone {
            store.connection().execute_unprepared("UPDATE runner_generations SET resources_destroyed_at=150,updated_at=150 WHERE id='expired';
                UPDATE runner_operations SET state='Succeeded',updated_at=150 WHERE id='destroy'").await?;
        }
        let report = store
            .diagnostics(SubjectKind::Generation, "expired", &actor(), 200)
            .await?
            .ok_or("generation diagnostics")?;
        let cleanup = report
            .questions
            .iter()
            .find(|q| q.question == QuestionId::Cleanup)
            .ok_or("cleanup question")?;
        assert_eq!(cleanup.basis.kind, BasisKind::LedgerProjection);
        assert_eq!(cleanup.cleanup_mode, Some(CleanupMode::HardLifetime));
        assert!(cleanup
            .reasons
            .iter()
            .any(|r| r.code == Code::CleanupHardLifetime.as_str()));
        let expected = if resource_gone {
            Code::CleanupRegistrationPending
        } else {
            Code::LifecycleApplyOutcomeUnknown
        };
        assert!(cleanup.reasons.iter().any(|r| r.code == expected.as_str()));
        assert_ne!(cleanup.outcome, Outcome::Satisfied);
    }
    Ok(())
}
