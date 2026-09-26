use super::*;

#[tokio::test]
async fn diagnostics_full_queue_invalidates_old_success_and_recovers(
) -> Result<(), Box<dyn std::error::Error>> {
    let (sender, mut receiver) = mpsc::channel(1024);
    let hub = Arc::new(Hub {
        entries: Mutex::new(HashMap::new()),
        sender,
        dropped: AtomicU64::new(0),
        health: AtomicU64::new(0),
        write_failures: AtomicU64::new(0),
    });
    let guard = Guard::fleet(
        "fleet",
        &shaula_core::registry::FleetRuntimeGuard {
            incarnation: "inc".into(),
            desired_revision: 1,
            mutation_fence: 1,
        },
    );
    let observer = Observer::register(
        Some(hub.clone()),
        guard,
        Lane::Supervisor,
        QuestionId::ScaleUp,
    )
    .ok_or("observer")?;
    for now in 0..1024 {
        observer.begin(now).ok_or("ticket")?.publish();
    }
    let overflow = observer.begin(1024).ok_or("ticket")?;
    let rejected = overflow.observation.clone();
    overflow.publish();
    assert!(!hub.current(&rejected));
    assert_eq!(hub.dropped.load(Ordering::Relaxed), 1);
    receiver.recv().await.ok_or("queued item")?;
    let recovery = observer.begin(1025).ok_or("ticket")?;
    let recovered = recovery.observation.clone();
    recovery.publish();
    assert!(hub.current(&recovered));
    Ok(())
}
