//! Ownership recovery and rejection cases using the production SQLite store.

use shaula_core::error::ReasonCode;
use shaula_core::github::Label;
use shaula_core::ports::{AccessFailure, EffectOutcome, LookupOutcome, ScaleSetView};
use shaula_core::registry::LifecycleStore;
use std::sync::atomic::Ordering;

use super::setup_with_labels;

fn label(name: &str, label_type: &str) -> Label {
    Label {
        name: name.into(),
        label_type: label_type.into(),
    }
}

#[tokio::test]
async fn blocked_default_label_binding_recovers_without_creating_another_scale_set() {
    let (store, github, supervisor, _) = setup_with_labels(Vec::new()).await;
    // The old comparison rejected the lowercase service response. The wire
    // adapter now supplies the canonical System type; a persisted AccessBlocked
    // row must recover against the same scale set, without a new create call.
    *github.labels.lock().unwrap() = Some(vec![label("shaula-x64", "System")]);
    assert!(supervisor.tick(9).await.unwrap().handoff_acknowledged);
    assert!(supervisor.tick(10).await.unwrap().scale_set_bound);
    let mut row = store.scale_set_get("f1").await.unwrap().unwrap();
    row.state = "AccessBlocked".into();
    row.now = 11;
    store.scale_set_upsert(row).await.unwrap();

    let recovered = supervisor.tick(20).await.unwrap();
    assert!(recovered.scale_set_bound);
    assert!(!recovered.blocked);
    let row = store.scale_set_get("f1").await.unwrap().unwrap();
    assert_eq!(row.state, "Adopted");
    assert_eq!(row.scale_set_id, Some(42));
    assert_eq!(github.effects.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn default_label_name_and_type_conflicts_remain_blocked() {
    for labels in [
        vec![label("different-fleet", "System")],
        vec![label("shaula-x64", "Customer")],
        vec![label("shaula-x64", "future-type")],
        Vec::new(),
    ] {
        let (store, github, supervisor, _) = setup_with_labels(Vec::new()).await;
        *github.labels.lock().unwrap() = Some(labels);
        assert!(supervisor.tick(9).await.unwrap().handoff_acknowledged);
        let report = supervisor.tick(10).await.unwrap();
        assert!(report.blocked);
        assert!(!report.scale_set_bound);
        assert_eq!(report.reason, Some(ReasonCode::OwnershipConflict));
        assert_eq!(
            store.scale_set_get("f1").await.unwrap().unwrap().state,
            "AccessBlocked"
        );
        assert_eq!(github.effects.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn access_failures_report_bounded_reasons_without_creating_resources() {
    for (failure, reason) in [
        (
            AccessFailure::PermissionDenied,
            ReasonCode::PermissionDenied,
        ),
        (
            AccessFailure::TargetHiddenOrNotFound,
            ReasonCode::TargetHiddenOrNotFound,
        ),
        (
            AccessFailure::RateLimited { retry_after: None },
            ReasonCode::RateLimited,
        ),
        (
            AccessFailure::Unavailable {
                summary: "private provider diagnostic".into(),
            },
            ReasonCode::AccessVerificationFailed,
        ),
    ] {
        let (_, github, supervisor, _) = setup_with_labels(Vec::new()).await;
        supervisor.tick(9).await.unwrap();
        *github.lookup_override.lock().unwrap() = Some(Err(failure));
        let report = supervisor.tick(10).await.unwrap();
        assert!(report.blocked);
        assert_eq!(report.reason, Some(reason));
        assert!(!format!("{report:?}").contains("private provider diagnostic"));
        assert_eq!(github.effects.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn incompatible_create_response_waits_for_authoritative_recovery() {
    let (store, github, supervisor, _) = setup_with_labels(Vec::new()).await;
    *github.labels.lock().unwrap() = Some(vec![label("shaula-x64", "System")]);
    supervisor.tick(9).await.unwrap();
    *github.lookup_override.lock().unwrap() = Some(Ok(LookupOutcome::None));
    *github.create_override.lock().unwrap() = Some(Ok(EffectOutcome::Definite(ScaleSetView {
        id: 42,
        name: "shaula-x64".into(),
        runner_group_id: 7,
        runner_group_name: "Default".into(),
        labels: vec![label("another-route", "System")],
    })));
    let report = supervisor.tick(10).await.unwrap();
    assert_eq!(report.reason, Some(ReasonCode::OwnershipConflict));
    assert!(!report.scale_set_bound);
    assert_eq!(github.effects.load(Ordering::SeqCst), 1);
    assert_eq!(
        store.scale_set_get("f1").await.unwrap().unwrap().state,
        "AccessBlocked"
    );

    *github.lookup_override.lock().unwrap() = None;
    assert!(supervisor.tick(20).await.unwrap().scale_set_bound);
    assert_eq!(github.effects.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn adopted_label_drift_blocks_until_the_exact_route_returns() {
    let (store, github, supervisor, _) = setup_with_labels(Vec::new()).await;
    *github.labels.lock().unwrap() = Some(vec![label("shaula-x64", "System")]);
    supervisor.tick(9).await.unwrap();
    assert!(supervisor.tick(10).await.unwrap().scale_set_bound);

    *github.labels.lock().unwrap() = Some(vec![label("different-fleet", "System")]);
    assert!(supervisor.tick(20).await.unwrap().blocked);
    assert_eq!(
        store.scale_set_get("f1").await.unwrap().unwrap().state,
        "AccessBlocked"
    );

    *github.labels.lock().unwrap() = Some(vec![label("shaula-x64", "System")]);
    assert!(supervisor.tick(30).await.unwrap().scale_set_bound);
    assert_eq!(
        store.scale_set_get("f1").await.unwrap().unwrap().state,
        "Adopted"
    );
    assert_eq!(github.effects.load(Ordering::SeqCst), 0);
}
