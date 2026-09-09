//! Bounded FIFO barrier before reading a completed apply's sanitized projection.
use super::{Event, CURRENT, FLUSH};

pub(crate) async fn flush() {
    let Ok(capture) = CURRENT.try_with(Clone::clone) else {
        return;
    };
    let (sender, receiver) = tokio::sync::oneshot::channel();
    if !matches!(
        tokio::time::timeout(FLUSH, async {
            capture.sender.send(Event::Flush(sender)).await.ok()?;
            receiver.await.ok()
        })
        .await,
        Ok(Some(()))
    ) {
        capture
            .counts
            .partial
            .store(true, std::sync::atomic::Ordering::Relaxed);
        tracing::warn!(
            reason = "flush_timeout",
            "apply projection capture incomplete"
        );
    }
}
