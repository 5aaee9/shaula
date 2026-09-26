use super::*;
async fn seed_waiting_online(store: &impl LifecycleStore, digest: &str) {
    store
        .generation_insert(GenerationRecord {
            id: "gen1".into(),
            fleet_key: "f1".into(),
            runner_name: "runner1".into(),
            generation_name: "generation1".into(),
            fleet_revision: 1,
            pool_member_key: None,
            template_profile_key: "k8s-linux".into(),
            template_revision: 1,
            template_artifact_digest: digest.into(),
            attestation_id: "att".into(),
            inputs_digest: "inputs".into(),
            state: G::CreatePending,
            github_runner_id: None,
            workspace_path: "unused".into(),
            created_at: 1,
            updated_at: 1,
        })
        .await
        .unwrap();
    store
        .generation_set_github_runner("gen1", 12, 2)
        .await
        .unwrap();
    for state in [G::Creating, G::WaitingOnline] {
        store.generation_advance("gen1", state, 3).await.unwrap();
    }
}

async fn generation_state(store: &impl LifecycleStore) -> G {
    store
        .generations_for_fleet("f1")
        .await
        .unwrap()
        .pop()
        .unwrap()
        .state
}

#[tokio::test]
async fn readiness_observed_online_runner_advences_waiting_generation_to_idle() {
    let (store, github, supervisor, digest) = setup().await;
    seed_waiting_online(store.as_ref(), &digest).await;
    supervisor.tick(5).await.unwrap(); // settle the Pending handoff.
    github
        .runners
        .lock()
        .unwrap()
        .push(shaula_core::ports::RunnerRef {
            id: 12,
            name: "runner1".into(),
            scale_set_id: 42,
            status: "online".into(),
        });
    // Busy gates runner removal, so retirement engages (proving Idle was
    // reached) without driving a gated destroy in this harness.
    github.busy.store(true, Ordering::SeqCst);
    supervisor.tick(30).await.unwrap();
    assert_eq!(generation_state(store.as_ref()).await, G::Retiring);
    assert_eq!(github.removals.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn readiness_timeout_moves_absent_runner_to_cleanup() {
    let (store, github, supervisor, digest) = setup().await;
    seed_waiting_online(store.as_ref(), &digest).await;
    supervisor.tick(5).await.unwrap();
    // No runner in the inventory (never registered, or an ephemeral JIT
    // runner that served its job and self-deregistered) and past the
    // grace period (spec 0024 §2).
    let late = 1 + shaula_daemon::supervisor::READINESS_TIMEOUT_MS + 1;
    supervisor.tick(late).await.unwrap();
    assert_eq!(generation_state(store.as_ref()).await, G::CleanupRequired);
    assert_eq!(github.removals.load(Ordering::SeqCst), 0);
}

/// 2026-09-12 incident: a JIT ephemeral runner observed online (Idle)
/// then served its job and self-deregistered. With demand still >= 1
/// (the next job already assigned to the scale set), the phantom Idle
/// generation held effective capacity forever: nothing created a
/// replacement and nothing destroyed the dead VM. The Idle inventory
/// re-check must advance it to Retiring so the removal/destroy chain
/// runs even at zero excess (rev 3, spec 0024).
#[tokio::test]
async fn idle_generation_whose_runner_vanished_from_inventory_retires() {
    let (store, github, supervisor, digest) = setup().await;
    seed_idle(store.as_ref(), &digest).await;
    supervisor.tick(5).await.unwrap(); // settle the Pending handoff.
                                       // Demand snapshot keeps target at 1 while the runner is already
                                       // gone from the inventory (self-deregistered after its single job).
    store.demand_snapshot("f1", 1, 6).await.unwrap();
    supervisor.tick(10).await.unwrap();
    // Idle -> Retiring, then the retirement chain re-gates the runner
    // removal at zero excess and attempts it once (the seeded row has
    // no Create provenance, so the destroy gate quarantines it rather
    // than reaching Terraform).
    assert_eq!(
        store.generation_get("gen1").await.unwrap().unwrap().state,
        G::Destroyed
    );
    assert_eq!(github.removals.load(Ordering::SeqCst), 1);
}

/// An Idle generation whose runner is still online in the inventory is
/// untouched: the phantom-runner re-check must never retire live
/// capacity.
#[tokio::test]
async fn idle_generation_with_online_runner_stays_idle() {
    let (store, github, supervisor, digest) = setup().await;
    seed_idle(store.as_ref(), &digest).await;
    supervisor.tick(5).await.unwrap(); // settle the Pending handoff.
    github
        .runners
        .lock()
        .unwrap()
        .push(shaula_core::ports::RunnerRef {
            id: 12,
            name: "runner1".into(),
            scale_set_id: 42,
            status: "online".into(),
        });
    store.demand_snapshot("f1", 1, 6).await.unwrap();
    supervisor.tick(10).await.unwrap();
    assert_eq!(
        store.generation_get("gen1").await.unwrap().unwrap().state,
        G::Idle
    );
    assert_eq!(github.removals.load(Ordering::SeqCst), 0);
}

/// An Idle generation whose runner is still registered but offline
/// (agent dead, unit is Restart=no) can never serve another job — it
/// retires rather than stranding capacity on a zombie registration.
#[tokio::test]
async fn idle_generation_with_offline_runner_retires() {
    let (store, github, supervisor, digest) = setup().await;
    seed_idle(store.as_ref(), &digest).await;
    supervisor.tick(5).await.unwrap(); // settle the Pending handoff.
    github
        .runners
        .lock()
        .unwrap()
        .push(shaula_core::ports::RunnerRef {
            id: 12,
            name: "runner1".into(),
            scale_set_id: 42,
            status: "offline".into(),
        });
    store.demand_snapshot("f1", 1, 6).await.unwrap();
    supervisor.tick(10).await.unwrap();
    assert_eq!(
        store.generation_get("gen1").await.unwrap().unwrap().state,
        G::Destroyed
    );
    assert_eq!(github.removals.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn readiness_keeps_young_absent_generation_waiting() {
    let (store, _github, supervisor, digest) = setup().await;
    seed_waiting_online(store.as_ref(), &digest).await;
    supervisor.tick(5).await.unwrap();
    // Within the boot grace: no transition, no removal, no cleanup.
    supervisor.tick(5 * 60 * 1000).await.unwrap();
    assert_eq!(generation_state(store.as_ref()).await, G::WaitingOnline);
}
