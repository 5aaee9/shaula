//! Own every worker until completion, including panic and cancellation paths.
use shaula_core::error::CoreResult;
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct FleetTasks {
    tasks: tokio::task::JoinSet<CoreResult<()>>,
    owners: HashMap<tokio::task::Id, String>,
}

impl FleetTasks {
    pub fn contains(&self, key: &str) -> bool {
        self.owners.values().any(|k| k == key)
    }
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }
    pub fn spawn(
        &mut self,
        key: String,
        task: impl std::future::Future<Output = CoreResult<()>> + Send + 'static,
    ) {
        let handle = self.tasks.spawn(task);
        self.owners.insert(handle.id(), key);
    }
    pub async fn join_next(&mut self) -> Option<(String, Result<(), String>)> {
        let (id, result) = match self.tasks.join_next_with_id().await? {
            Ok((id, result)) => (id, result.map_err(|e| e.summary)),
            Err(e) => (e.id(), Err("fleet worker panicked or was cancelled".into())),
        };
        self.owners.remove(&id).map(|key| (key, result))
    }
    pub async fn shutdown(&mut self) {
        self.tasks.abort_all();
        while self.join_next().await.is_some() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn slow_and_panicking_fleets_do_not_stop_other_workers() {
        let mut workers = FleetTasks::default();
        workers.spawn("slow".into(), std::future::pending());
        workers.spawn("failed".into(), async {
            std::panic::resume_unwind(Box::new("test panic"))
        });
        workers.spawn("healthy".into(), async { Ok(()) });
        for _ in 0..2 {
            let joined =
                tokio::time::timeout(std::time::Duration::from_secs(2), workers.join_next()).await;
            assert!(matches!(joined, Ok(Some(_))));
        }
        assert!(workers.contains("slow"));
        assert!(!workers.contains("healthy"));
        assert!(!workers.contains("failed"));
        workers.shutdown().await;
        assert!(workers.is_empty());
        assert!(!workers.contains("slow"));
    }
}
