use super::*;
use async_trait::async_trait;
use shaula_core::error::{CoreError, CoreResult, ReasonCode};

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct SlowArchive {
    first_append: AtomicBool,
    release: tokio::sync::Semaphore,
    texts: Mutex<Vec<String>>,
    result: Mutex<Option<FinishInvocation>>,
}

#[tokio::test]
async fn flush_waits_for_queued_apply_text_without_sealing_invocation() -> TestResult {
    let sink = Arc::new(SlowArchive::default());
    let tasks = CaptureTasks::default();
    let guard = InvocationGuard::begin(
        Some(sink.clone()),
        &tasks,
        "generation",
        "Create",
        SensitiveValues::default(),
    )
    .await
    .ok_or("capture unavailable")?;
    guard
        .scope(async {
            let mut command = command("apply").ok_or("apply capture missing")?;
            let mut pipe = command.pipe("stdout");
            pipe.feed(b"Apply complete! Resources: 1 added, 0 changed, 0 destroyed.\n");
            pipe.finish(true);
            command.finish(Some(0), "exited");
            let barrier = flush();
            tokio::pin!(barrier);
            assert!(
                tokio::time::timeout(Duration::from_millis(10), &mut barrier)
                    .await
                    .is_err()
            );
            sink.release.add_permits(1);
            barrier.await;
            assert!(!sink
                .texts
                .lock()
                .map_err(|_| "text lock poisoned")?
                .is_empty());
            assert!(sink
                .result
                .lock()
                .map_err(|_| "result lock poisoned")?
                .is_none());
            Ok::<(), Box<dyn std::error::Error>>(())
        })
        .await?;
    guard.finish("succeeded").await;
    tasks.drain().await;
    Ok(())
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
            self.release
                .acquire()
                .await
                .map_err(|_| CoreError::new(ReasonCode::Internal, "test release closed"))?
                .forget();
        }
        self.texts
            .lock()
            .map_err(|_| CoreError::new(ReasonCode::Internal, "test text lock poisoned"))?
            .push(chunk.text);
        Ok(())
    }

    async fn command(&self, _: LogCommand) -> CoreResult<()> {
        Ok(())
    }

    async fn finish(&self, result: FinishInvocation) -> CoreResult<()> {
        *self
            .result
            .lock()
            .map_err(|_| CoreError::new(ReasonCode::Internal, "test result lock poisoned"))? =
            Some(result);
        Ok(())
    }
}

#[tokio::test]
async fn full_queue_discards_only_sanitized_records_and_keeps_failure_outcome() -> TestResult {
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
    .ok_or("capture did not start")?;
    guard
        .scope(async {
            let mut command = command("apply").ok_or("apply capture missing")?;
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
            Ok::<(), &'static str>(())
        })
        .await?;
    sink.release.add_permits(1);
    guard.finish("failed").await;
    tasks.drain().await;
    let result = sink.result.lock().map_err(|_| "result lock poisoned")?;
    let result = result.as_ref().ok_or("capture did not finalize")?;
    assert_eq!(result.execution_outcome, "failed");
    assert!(result.partial);
    assert!(result.lost_bytes > 0);
    let text = sink
        .texts
        .lock()
        .map_err(|_| "text lock poisoned")?
        .join("");
    assert!(!text.contains("abc"));
    assert!(!text.contains("def"));
    assert!(text.contains("Error:"));
    assert!(text.contains("Destroying..."));
    Ok(())
}

#[tokio::test]
async fn cancelled_capture_closes_with_partial_interrupted_outcome() -> TestResult {
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
    .ok_or("capture did not start")?;
    guard
        .scope(async {
            let command = command("apply").ok_or("apply capture missing")?;
            let mut pipe = command.pipe("stdout");
            pipe.feed(b"Error: unterminated-credential");
            Ok::<(), &'static str>(())
        })
        .await?;
    drop(guard);
    tasks.drain().await;
    let result = sink.result.lock().map_err(|_| "result lock poisoned")?;
    let result = result.as_ref().ok_or("capture did not finalize")?;
    assert_eq!(result.execution_outcome, "interrupted");
    assert!(result.partial);
    let text = sink
        .texts
        .lock()
        .map_err(|_| "text lock poisoned")?
        .join("");
    assert_eq!(text, crate::operation_sanitize::WITHHELD);
    Ok(())
}
