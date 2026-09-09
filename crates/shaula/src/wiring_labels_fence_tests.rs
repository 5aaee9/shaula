//! Authority can change while remote ownership reads are outstanding.

use super::*;
use tokio::sync::Notify;

async fn pause_inventory(
    wiring: &mut SupervisorWiring,
    listener: &ListenerMock,
) -> (
    Arc<Notify>,
    tokio::task::JoinHandle<
        shaula_core::error::CoreResult<shaula_daemon::supervisor::ReconcileReport>,
    >,
) {
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    *listener.inventory_barrier.lock().unwrap() = Some((entered.clone(), release.clone()));
    *listener.label_values.lock().unwrap() = Some(serde_json::json!([
        {"name":"drift","type":"User"}
    ]));
    let head = wiring.store.fleet_get(FLEET).await.unwrap().unwrap();
    let supervisor = wiring
        .supervisor_for(FLEET, head.desired_revision, &head.phase)
        .await
        .unwrap()
        .unwrap();
    let task = tokio::spawn(async move { supervisor.tick(NOW + 5_000).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    (release, task)
}

#[tokio::test]
async fn labels_effect_gate_serializes_replacement_through_remote_readback() {
    let (plane, mut wiring, listener, clock, _) = setup().await;
    ready(&plane, &mut wiring, &clock).await;
    let (release, task) = pause_inventory(&mut wiring, &listener).await;
    let ownership = plane
        .control_plane
        .scale_set_get(FLEET)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ownership.state, "LabelsUpdating");
    let exclusive = wiring.gates.acquire_exclusive(FLEET);
    tokio::pin!(exclusive);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), &mut exclusive)
            .await
            .is_err(),
        "replacement must wait for the in-flight effect"
    );
    release.notify_one();
    let permit = tokio::time::timeout(std::time::Duration::from_secs(5), exclusive)
        .await
        .unwrap();
    assert_eq!(listener.label_updates.load(Ordering::SeqCst), 1);
    assert_eq!(
        plane
            .control_plane
            .scale_set_get(FLEET)
            .await
            .unwrap()
            .unwrap()
            .state,
        "Adopted"
    );
    drop(permit);
    assert!(task.await.unwrap().unwrap().scale_set_bound);
    wiring.tasks.shutdown().await;
}

#[tokio::test]
async fn labels_recheck_current_revision_deletion_and_auth_after_inventory() {
    use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
    for invalidation in ["revision", "deletion", "auth"] {
        let (plane, mut wiring, listener, clock, _) = setup().await;
        ready(&plane, &mut wiring, &clock).await;
        let (release, task) = pause_inventory(&mut wiring, &listener).await;
        // Direct durable mutations deliberately bypass admission's gate to
        // model an authority change during an awaited remote proof. Production
        // PUT/DELETE also serialize, as asserted in the preceding test.
        match invalidation {
            "revision" => {
                labels_tests::commit_labels(&plane, &["newer"], NOW + 6_000).await;
            }
            other => {
                let db = Database::connect(format!("sqlite://{}?mode=rw", plane.db_path.display()))
                    .await
                    .unwrap();
                let sql = if other == "deletion" {
                    "UPDATE fleets SET deletion_marker=1, mutation_fence=mutation_fence+1 WHERE key='f1'"
                } else {
                    "UPDATE fleet_auth_contexts SET state='Pending' WHERE fleet_key='f1'"
                };
                db.execute(Statement::from_string(DbBackend::Sqlite, sql))
                    .await
                    .unwrap();
                db.close().await.unwrap();
            }
        }
        release.notify_one();
        let report = tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(report.blocked, "{invalidation}: {report:?}");
        assert_eq!(
            listener.label_updates.load(Ordering::SeqCst),
            0,
            "{invalidation}"
        );
        assert_eq!(
            plane
                .control_plane
                .scale_set_get(FLEET)
                .await
                .unwrap()
                .unwrap()
                .state,
            "LabelsUpdating"
        );
        wiring.tasks.shutdown().await;
    }
}
