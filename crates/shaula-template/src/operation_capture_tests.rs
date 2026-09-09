use super::*;
use async_trait::async_trait;
use shaula_core::error::CoreResult;

struct SlowArchive {
    first_append: AtomicBool,
    release: tokio::sync::Semaphore,
    texts: Mutex<Vec<String>>,
    result: Mutex<Option<FinishInvocation>>,
}

impl Default for SlowArchive {
    fn default() -> Self {
        Self {
            first_append: AtomicBool::new(true),
            release: tokio::sync::Semaphore::new(0),
            texts: Mutex::new(Vec::new()),
            result: Mutex::new(None),
        }
    }
}

#[async_trait]
impl OperationLogSink for SlowArchive {
    async fn begin(&self, _: BeginInvocation) -> CoreResult<String> {
        Ok(uuid::Uuid::new_v4().to_string())
    }

    async fn append(&self, chunk: AppendLog) -> CoreResult<()> {
        if self.first_append.swap(false, Ordering::SeqCst) {
            self.release.acquire().await.expect("test release").forget();
        }
        self.texts.lock().expect("test text lock").push(chunk.text);
        Ok(())
    }

    async fn command(&self, _: LogCommand) -> CoreResult<()> {
        Ok(())
    }

    async fn finish(&self, result: FinishInvocation) -> CoreResult<()> {
        *self.result.lock().expect("test result lock") = Some(result);
        Ok(())
    }
}

#[tokio::test]
async fn full_queue_discards_only_sanitized_records_and_keeps_failure_outcome() {
    let sink = Arc::new(SlowArchive::default());
    let tasks = CaptureTasks::default();
    let values = SensitiveValues::from_input(&serde_json::json!({"jit_config":"abcdef"}), &[]);
    let guard = InvocationGuard::begin(
        Some(sink.clone()),
        &tasks,
        &uuid::Uuid::new_v4().to_string(),
        "Destroy",
        values,
    )
    .await
    .expect("capture starts");
    guard
        .scope(async {
            let mut command = command("apply").expect("apply capture");
            let mut pipe = command.pipe("stderr");
            pipe.feed(b"Error: abc");
            pipe.feed(b"def\n");
            pipe.flush();
            tokio::task::yield_now().await;
            for _ in 0..QUEUE_CHUNKS * 3 {
                pipe.feed(b"docker_container.runner: Destroying...\n");
                pipe.flush();
            }
            pipe.finish(true);
            command.finish(Some(1), "exited");
        })
        .await;
    sink.release.add_permits(1);
    guard.finish("failed").await;
    tasks.drain().await;
    let result = sink.result.lock().expect("result lock");
    let result = result.as_ref().expect("capture finalized");
    assert_eq!(result.execution_outcome, "failed");
    assert!(result.partial);
    assert!(result.lost_bytes > 0);
    let text = sink.texts.lock().expect("text lock").join("");
    assert!(!text.contains("abc"));
    assert!(!text.contains("def"));
    assert!(text.contains("Error:"));
    assert!(text.contains("Destroying..."));
}

#[tokio::test]
async fn cancelled_capture_closes_with_partial_interrupted_outcome() {
    let sink = Arc::new(SlowArchive::default());
    sink.release.add_permits(1);
    let tasks = CaptureTasks::default();
    let guard = InvocationGuard::begin(
        Some(sink.clone()),
        &tasks,
        &uuid::Uuid::new_v4().to_string(),
        "Create",
        SensitiveValues::default(),
    )
    .await
    .expect("capture starts");
    guard
        .scope(async {
            let command = command("apply").expect("apply capture");
            let mut pipe = command.pipe("stdout");
            pipe.feed(b"Error: unterminated-credential");
        })
        .await;
    drop(guard);
    tasks.drain().await;
    let result = sink.result.lock().expect("result lock");
    let result = result.as_ref().expect("capture finalized");
    assert_eq!(result.execution_outcome, "interrupted");
    assert!(result.partial);
    let text = sink.texts.lock().expect("text lock").join("");
    assert_eq!(text, crate::operation_sanitize::WITHHELD);
}
