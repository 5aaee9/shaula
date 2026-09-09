//! Nonblocking, already-sanitized pipe capture with bounded terminal flushing.
use crate::operation_sanitize::{Sanitizer, SensitiveValues};
use shaula_core::operation_log::*;
use std::sync::{
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

tokio::task_local! { static CURRENT: Capture; }

const QUEUE_CHUNKS: usize = 64;
const CHUNK_BYTES: usize = 16 * 1024;
const FLUSH: Duration = Duration::from_secs(5);

pub(crate) fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

enum Event {
    Text(AppendLog),
    Command(LogCommand),
    Flush(oneshot::Sender<()>),
    Finish(String, oneshot::Sender<()>),
}

struct Counts {
    lost: AtomicU64,
    partial: AtomicBool,
}

#[derive(Clone)]
pub(crate) struct Capture {
    id: String,
    sender: mpsc::Sender<Event>,
    counts: Arc<Counts>,
    sequence: Arc<Mutex<u64>>,
    command: Arc<AtomicU32>,
    secrets: Arc<SensitiveValues>,
    effect: Arc<Mutex<Option<String>>>,
}

#[derive(Default)]
pub(crate) struct CaptureTasks(Mutex<Vec<tokio::task::JoinHandle<()>>>);

impl CaptureTasks {
    pub(crate) async fn drain(&self) {
        let tasks = self
            .0
            .lock()
            .map(|mut tasks| std::mem::take(&mut *tasks))
            .unwrap_or_default();
        for mut task in tasks {
            if tokio::time::timeout(FLUSH, &mut task).await.is_err() {
                task.abort();
                let _ = task.await;
            }
        }
    }
}

pub(crate) struct InvocationGuard {
    capture: Capture,
}

impl InvocationGuard {
    pub(crate) async fn begin(
        sink: Option<Arc<dyn OperationLogSink>>,
        tasks: &CaptureTasks,
        generation: &str,
        operation: &str,
        values: SensitiveValues,
    ) -> Option<Self> {
        let sink = sink?;
        let id = match tokio::time::timeout(
            Duration::from_secs(1),
            sink.begin(BeginInvocation {
                generation_id: generation.into(),
                operation: operation.into(),
                started_at: now(),
            }),
        )
        .await
        {
            Ok(Ok(id)) => id,
            _ => {
                tracing::warn!(
                    reason = "archive_unavailable",
                    "operation log capture unavailable"
                );
                return None;
            }
        };
        let (sender, mut receiver) = mpsc::channel(QUEUE_CHUNKS);
        let counts = Arc::new(Counts {
            lost: AtomicU64::new(0),
            partial: AtomicBool::new(false),
        });
        let state = counts.clone();
        let invocation = id.clone();
        let task = tokio::spawn(async move {
            let mut outcome = "interrupted".to_string();
            let mut acknowledgement = None;
            while let Some(event) = receiver.recv().await {
                let result = match event {
                    Event::Text(text) => {
                        let bytes = text.text.len() as u64;
                        let result =
                            tokio::time::timeout(Duration::from_secs(2), sink.append(text)).await;
                        if !matches!(result, Ok(Ok(()))) {
                            state.lost.fetch_add(bytes, Ordering::Relaxed);
                        }
                        matches!(result, Ok(Ok(())))
                    }
                    Event::Command(mut command) => {
                        command.capture_partial |= state.partial.load(Ordering::Relaxed);
                        command.lost_bytes = state.lost.load(Ordering::Relaxed);
                        matches!(
                            tokio::time::timeout(Duration::from_secs(2), sink.command(command))
                                .await,
                            Ok(Ok(()))
                        )
                    }
                    Event::Finish(result, ack) => {
                        outcome = result;
                        acknowledgement = Some(ack);
                        break;
                    }
                    Event::Flush(ack) => {
                        let _ = ack.send(());
                        true
                    }
                };
                if !result {
                    state.partial.store(true, Ordering::Relaxed);
                }
            }
            let partial = state.partial.load(Ordering::Relaxed) || outcome == "interrupted";
            let result = FinishInvocation {
                invocation_id: invocation,
                execution_outcome: outcome,
                ended_at: now(),
                lost_bytes: state.lost.load(Ordering::Relaxed),
                partial,
            };
            if !matches!(
                tokio::time::timeout(FLUSH, sink.finish(result)).await,
                Ok(Ok(()))
            ) {
                tracing::warn!(
                    reason = "archive_unavailable",
                    "operation log finalization unavailable"
                );
            }
            if let Some(ack) = acknowledgement {
                let _ = ack.send(());
            }
        });
        if let Ok(mut pending) = tasks.0.lock() {
            pending.retain(|task| !task.is_finished());
            pending.push(task);
        }
        Some(Self {
            capture: Capture {
                id,
                sender,
                counts,
                sequence: Arc::new(Mutex::new(0)),
                command: Arc::new(AtomicU32::new(0)),
                secrets: Arc::new(values),
                effect: Arc::new(Mutex::new(None)),
            },
        })
    }

    pub(crate) async fn scope<T>(&self, future: impl std::future::Future<Output = T>) -> T {
        CURRENT.scope(self.capture.clone(), future).await
    }

    pub(crate) async fn finish(self, outcome: &str) {
        let (sender, receiver) = oneshot::channel();
        if tokio::time::timeout(FLUSH, async {
            let _ = self
                .capture
                .sender
                .send(Event::Finish(outcome.into(), sender))
                .await;
            let _ = receiver.await;
        })
        .await
        .is_err()
        {
            self.capture.counts.partial.store(true, Ordering::Relaxed);
            tracing::warn!(
                reason = "flush_timeout",
                "operation log finalization incomplete"
            );
        }
    }
}

pub(crate) fn bind_effect(id: &str) {
    let _ = CURRENT.try_with(|capture| {
        if let Ok(mut effect) = capture.effect.lock() {
            *effect = Some(id.into());
        }
    });
}

pub(crate) fn command(phase: &str) -> Option<CommandCapture> {
    if !matches!(phase, "init" | "plan" | "apply") {
        return None;
    }
    CURRENT
        .try_with(|capture| {
            let ordinal = capture.command.fetch_add(1, Ordering::Relaxed);
            let descriptor = LogCommand {
                invocation_id: capture.id.clone(),
                ordinal,
                phase: phase.into(),
                started_at: now(),
                ended_at: None,
                exit_code: None,
                termination: "running".into(),
                effect_attempt_id: capture.effect.lock().ok().and_then(|v| v.clone()),
                capture_partial: capture.counts.partial.load(Ordering::Relaxed),
                lost_bytes: capture.counts.lost.load(Ordering::Relaxed),
            };
            capture.event(Event::Command(descriptor.clone()), 0);
            CommandCapture {
                capture: capture.clone(),
                descriptor,
                finished: false,
            }
        })
        .ok()
}

impl Capture {
    fn event(&self, event: Event, bytes: u64) {
        if self.sender.try_send(event).is_err() {
            self.counts.lost.fetch_add(bytes, Ordering::Relaxed);
            self.counts.partial.store(true, Ordering::Relaxed);
        }
    }
}

pub(crate) struct CommandCapture {
    capture: Capture,
    descriptor: LogCommand,
    finished: bool,
}

impl CommandCapture {
    pub(crate) fn pipe(&self, stream: &str) -> PipeCapture {
        PipeCapture {
            capture: self.capture.clone(),
            ordinal: self.descriptor.ordinal,
            phase: self.descriptor.phase.clone(),
            stream: stream.into(),
            sanitizer: Sanitizer::new(self.capture.secrets.clone()),
            pending: String::new(),
            withheld: false,
            finished: false,
        }
    }

    pub(crate) fn finish(&mut self, exit_code: Option<i32>, termination: &str) {
        self.descriptor.exit_code = exit_code;
        self.descriptor.ended_at = Some(now());
        self.descriptor.termination = termination.into();
        self.capture
            .event(Event::Command(self.descriptor.clone()), 0);
        self.finished = true;
    }
}

impl Drop for CommandCapture {
    fn drop(&mut self) {
        if !self.finished {
            self.capture.counts.partial.store(true, Ordering::Relaxed);
            self.finish(None, "interrupted");
        }
    }
}

pub(crate) struct PipeCapture {
    capture: Capture,
    ordinal: u32,
    phase: String,
    stream: String,
    sanitizer: Sanitizer,
    pending: String,
    withheld: bool,
    finished: bool,
}

impl PipeCapture {
    pub(crate) fn feed(&mut self, bytes: &[u8]) {
        for (text, withheld) in self.sanitizer.feed(bytes) {
            self.push(&text, withheld);
        }
    }

    fn push(&mut self, text: &str, withheld: bool) {
        if self.pending.len() + text.len() > CHUNK_BYTES {
            self.flush();
        }
        self.pending.push_str(text);
        self.withheld |= withheld;
        if self.pending.len() >= CHUNK_BYTES {
            self.flush();
        }
    }

    pub(crate) fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let Ok(mut sequence) = self.capture.sequence.lock() else {
            self.pending.clear();
            self.capture.counts.partial.store(true, Ordering::Relaxed);
            return;
        };
        let text = std::mem::take(&mut self.pending);
        let bytes = text.len() as u64;
        let chunk = AppendLog {
            invocation_id: self.capture.id.clone(),
            command_ordinal: self.ordinal,
            phase: self.phase.clone(),
            stream: self.stream.clone(),
            sequence: *sequence,
            text,
            observed_at: now(),
            withheld: self.withheld,
        };
        *sequence = sequence.saturating_add(1);
        self.withheld = false;
        self.capture.event(Event::Text(chunk), bytes);
    }

    pub(crate) fn finish(&mut self, complete: bool) {
        if self.finished {
            return;
        }
        if let Some((text, withheld)) = self.sanitizer.finish(complete) {
            self.push(&text, withheld);
        }
        if !complete {
            self.capture.counts.partial.store(true, Ordering::Relaxed);
        }
        self.flush();
        self.finished = true;
    }
}

impl Drop for PipeCapture {
    fn drop(&mut self) {
        self.finish(false);
    }
}

#[cfg(test)]
#[path = "operation_capture_tests.rs"]
mod tests;

#[path = "operation_capture_barrier.rs"]
mod barrier;
pub(crate) use barrier::flush;
