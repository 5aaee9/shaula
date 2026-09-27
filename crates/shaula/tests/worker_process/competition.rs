//! LW-12: Create-start against Fleet DELETE and Auth Handoff. Both take the
//! Fleet's exclusive effect gate (`acquire_exclusive` / `handoff_gate`); a
//! worker's ApplyStarting holds the shared claim across IPC until the daemon
//! resolves its spawn handover.
use super::fixture::*;
use std::{sync::Arc, time::Duration};
use tokio::{sync::Notify, time::timeout};

#[tokio::test]
#[ignore = "requires SHAULA_TEST_CGROUP and SHAULA_TEST_TERRAFORM"]
async fn unresolved_create_handover_blocks_delete_and_auth_handoff() -> TestResult {
    let fixture = Fixture::new().await?;
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    *fixture
        .control_faults
        .hold_spawned
        .lock()
        .map_err(|_| "hold poisoned")? = Some((entered.clone(), release.clone()));
    let gates = fixture.sink.gates.clone();
    let probe = async {
        timeout(Duration::from_secs(120), entered.notified()).await?;
        // The apply may already run, and the handover is durable.
        assert_eq!(fixture.handover().await?, (true, true));
        let exclusive = gates.acquire_exclusive("fleet");
        tokio::pin!(exclusive);
        assert!(
            timeout(Duration::from_millis(300), &mut exclusive)
                .await
                .is_err(),
            "DELETE/Auth Handoff must wait for the unresolved spawn handover"
        );
        release.notify_one();
        let permit = timeout(Duration::from_secs(30), exclusive).await?;
        assert_eq!(fixture.handover().await?, (false, true));
        drop(permit);
        TestResult::Ok(())
    };
    let (created, probed) = tokio::join!(fixture.create(), probe);
    probed?;
    created?;
    assert_eq!(fixture.create_count().await?, 1);
    fixture.close().await
}

#[tokio::test]
#[ignore = "requires SHAULA_TEST_CGROUP and SHAULA_TEST_TERRAFORM"]
async fn create_start_after_delete_commit_never_spawns_apply() -> TestResult {
    let fixture = Fixture::new().await?;
    fixture.commit_delete().await?;
    assert!(
        fixture.create().await.is_err(),
        "a stale Create must be refused at the durable apply boundary"
    );
    assert_eq!(fixture.create_count().await?, 0);
    assert_eq!(fixture.handover().await?, (false, false));
    // No Terraform apply ran, so no managed resource can exist in state.
    assert!(fixture
        .state()
        .await
        .map_or(true, |state| state.managed_empty()));
    fixture.close().await
}
