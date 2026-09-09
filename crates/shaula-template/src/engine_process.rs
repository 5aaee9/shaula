//! Spawned-engine ownership (R6-04): a process-tree fence, abort-on-drop
//! pipe readers, and the spawn/wait split that lets the runtime release
//! the short admission claim at the SPAWN HANDOVER instead of holding it
//! for the child's whole lifetime (R6-03).

use std::time::Duration;

use tokio::process::{Child, Command};

use shaula_core::error::{CoreError, CoreResult, ReasonCode};

use super::{logged_drain, ProcessOutput};

fn err(code: ReasonCode, msg: impl Into<String>) -> CoreError {
    CoreError::new(code, msg)
}

/// Abort-on-drop pipe reader: a cancelled engine future must not leave
/// detached reader tasks behind — dropping the guard terminates the
/// reader; the normal wait path joins through the Deref first (C12/C13
/// semantics preserved).
pub struct DrainGuard(tokio::task::JoinHandle<std::io::Result<Vec<u8>>>);

impl Drop for DrainGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl std::ops::Deref for DrainGuard {
    type Target = tokio::task::JoinHandle<std::io::Result<Vec<u8>>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for DrainGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// The process-tree fence (R6-04, R7-01, R9-09; spec 0001 §491):
/// OS-enforced termination of the WHOLE descendant tree when the guard
/// drops — including on cancellation — and, via the supervisor's stdin
/// watchdog, on daemon process death (even SIGKILL, where no Drop runs).
/// Established BEFORE any engine code can run on every platform; the
/// direct child is ALWAYS the fence supervisor (`engine_supervisor`):
///
/// Unix: the supervisor is created as its OWN PROCESS GROUP leader
/// pre-exec via the standard library's `process_group(0)` (spawn FAILS
/// if that cannot be established — no post-exec adoption and no
/// fake-success fence, R7-01). The engine inherits the group, so the
/// guard drop SIGKILLs the supervisor plus the whole tree; the
/// supervisor's own watchdog covers the daemon-death window.
///
/// Windows: the fence lives INSIDE the supervisor — the engine is a
/// BORN member of its kill-on-close job, and the supervisor's exit
/// (daemon kill, crash or daemon death) is what closes the job. Nothing
/// to hold here (R7-06).
enum ProcessTreeGuard {
    #[cfg(unix)]
    Group(nix::unistd::Pid),
    #[cfg(windows)]
    Delegated,
    #[cfg(not(any(windows, unix)))]
    Unavailable,
}

impl ProcessTreeGuard {
    fn establish(child: &Child) -> CoreResult<Self> {
        #[cfg(unix)]
        {
            use nix::unistd::Pid;
            let Some(pid) = child.id() else {
                return Err(err(
                    ReasonCode::TemplateExecutionFailed,
                    "engine child pid missing",
                ));
            };
            Ok(Self::Group(Pid::from_raw(pid as i32)))
        }
        #[cfg(windows)]
        {
            let _ = child;
            Ok(Self::Delegated)
        }
        #[cfg(not(any(windows, unix)))]
        {
            let _ = child;
            Ok(Self::Unavailable)
        }
    }
}

impl Drop for ProcessTreeGuard {
    fn drop(&mut self) {
        match self {
            #[cfg(unix)]
            Self::Group(pgid) => {
                // Anything still alive in the group dies with the fence.
                use nix::sys::signal::{killpg, Signal};
                let _ = killpg(*pgid, Signal::SIGKILL);
            }
            // The delegated supervisor fence acts on the supervisor's
            // exit, not on the guard drop (R7-06).
            #[cfg(windows)]
            Self::Delegated => {}
            #[cfg(not(any(windows, unix)))]
            Self::Unavailable => {}
        }
    }
}

/// One spawned engine process under its OWN fence. Dropping it kills
/// the tree; [`EngineSpawn::wait`] is the proven-termination path.
pub struct EngineSpawn {
    child: Child,
    stdout: DrainGuard,
    stderr: DrainGuard,
    tree: Option<ProcessTreeGuard>,
    timeout: Duration,
    /// The supervisor's stdin WRITE end (R7-06/R9-09). Kept by the
    /// spawn — NOT by tokio's `Child`, whose `wait()` closes stdin — so
    /// the supervisor's daemon-death watchdog sees EOF only when THIS
    /// process dies or the spawn is finished, never while the engine
    /// runs.
    _supervisor_stdin: Option<tokio::process::ChildStdin>,
    capture: Option<crate::operation_capture::CommandCapture>,
}

impl EngineSpawn {
    /// Waits for process exit WITHIN the command timeout and joins both
    /// pipe readers — the result is STAGED (C13): it propagates only
    /// after both drains have been fenced. The TREE FENCE is closed as
    /// soon as the direct child has exited (R6-04): descendants that
    /// inherited the pipe write ends are terminated at that boundary,
    /// so the call's return IS the tree's termination proof and no
    /// descendant effect can land after (or drag out) the call.
    pub async fn wait(mut self) -> CoreResult<ProcessOutput> {
        let outcome = tokio::time::timeout(self.timeout, self.child.wait()).await;
        let staged: CoreResult<std::process::ExitStatus> = match outcome {
            Ok(Ok(status)) => Ok(status),
            Ok(Err(e)) => {
                tracing::warn!(summary = %e, "engine wait failed; fencing pipes");
                Err(err(
                    ReasonCode::TemplateExecutionFailed,
                    "engine wait failed",
                ))
            }
            Err(_) => {
                // Cancellation is not proof the child did not start; kill
                // and AWAIT it so termination is proven, then classify as
                // failure. The tree fence below covers descendants.
                if let Err(e) = self.child.kill().await {
                    tracing::warn!(summary = %e, "engine kill after timeout failed");
                }
                let _ = self.child.wait().await;
                Err(err(
                    ReasonCode::TemplateExecutionFailed,
                    "engine invocation timed out",
                ))
            }
        };
        // Close the fence NOW: the direct child has exited, so every
        // remaining process in the tree (e.g. a detached grandchild
        // holding the pipe write ends) is terminated and the readers
        // reach EOF immediately instead of babysitting strays.
        if let Some(tree) = self.tree.take() {
            drop(tree);
        }
        async fn join_drain(
            task: &mut tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
        ) -> CoreResult<Vec<u8>> {
            // The child has exited, so EOF normally arrives immediately; a
            // grandchild inheriting the write end bounds via abort (our
            // read end closes, further writes fail with EPIPE).
            match tokio::time::timeout(Duration::from_secs(5), &mut *task).await {
                Ok(joined) => joined
                    .map_err(|e| {
                        err(
                            ReasonCode::TemplateExecutionFailed,
                            format!("engine reader failed: {e}"),
                        )
                    })?
                    .map_err(|e| {
                        err(
                            ReasonCode::TemplateExecutionFailed,
                            format!("engine output read failed: {e}"),
                        )
                    }),
                Err(_) => {
                    // Abort AND await: the cancelled reader's termination
                    // is proven before this call returns.
                    task.abort();
                    let _ = (&mut *task).await;
                    Err(err(
                        ReasonCode::TemplateExecutionFailed,
                        "engine output pipes did not close after exit",
                    ))
                }
            }
        }
        // Join BOTH drains even when the first fails: the failing `?`
        // result is produced only after the sibling has been fenced too.
        let drains =
            futures::future::join(join_drain(&mut self.stdout), join_drain(&mut self.stderr));
        let (stdout_res, stderr_res) = drains.await;
        if let Some(capture) = &mut self.capture {
            let code = staged
                .as_ref()
                .ok()
                .and_then(std::process::ExitStatus::code);
            let termination = if stdout_res.is_err() || stderr_res.is_err() {
                "reader_error"
            } else if staged
                .as_ref()
                .err()
                .is_some_and(|error| error.summary.contains("timed out"))
            {
                "timed_out"
            } else if staged.is_err() {
                "interrupted"
            } else {
                "exited"
            };
            capture.finish(code, termination);
        }
        let stdout_bytes = stdout_res?;
        let stderr_bytes = stderr_res?;
        let status = staged?;
        Ok(ProcessOutput {
            exit_code: status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
            stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
        })
    }
}

/// Spawns the command under a fresh process-tree fence with bounded
/// concurrent pipe readers (the child fills one pipe while the parent
/// blocks on the other would deadlock, C12).
pub(crate) fn spawn_fenced_logged(
    command: &mut Command,
    timeout: Duration,
    mut capture: Option<crate::operation_capture::CommandCapture>,
) -> CoreResult<EngineSpawn> {
    // R7-01: on Unix the child joins its OWN process group inside the
    // child itself, BEFORE exec — spawn fails if that cannot be
    // established, so a `Group` guard can never refer to a group the
    // child is not actually in.
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().map_err(|e| {
        if let Some(capture) = &mut capture {
            capture.finish(None, "not_spawned");
        }
        err(
            ReasonCode::TemplateExecutionFailed,
            format!("engine spawn failed: {e}"),
        )
    })?;
    // The fence is established BEFORE any reader starts and before the
    // spawn result escapes: from here on, every exit path (return,
    // error, cancellation, daemon death) terminates the tree.
    let tree = ProcessTreeGuard::establish(&child)?;
    // Keep the supervisor's stdin write end OURSELVES: tokio's
    // `Child::wait` closes stdin to unblock the child, which would make
    // the supervisor's daemon-death watchdog fire on every ordinary wait
    // (R7-06/R9-09). Held on the spawn, EOF now happens only when this
    // process dies or the invocation is finished.
    let supervisor_stdin = child.stdin.take();
    let Some(stdout_pipe) = child.stdout.take() else {
        return Err(err(
            ReasonCode::TemplateExecutionFailed,
            "child stdout missing",
        ));
    };
    let Some(stderr_pipe) = child.stderr.take() else {
        return Err(err(
            ReasonCode::TemplateExecutionFailed,
            "child stderr missing",
        ));
    };
    let stdout = DrainGuard(tokio::spawn(logged_drain(
        stdout_pipe,
        capture.as_ref().map(|c| c.pipe("stdout")),
    )));
    let stderr = DrainGuard(tokio::spawn(logged_drain(
        stderr_pipe,
        capture.as_ref().map(|c| c.pipe("stderr")),
    )));
    Ok(EngineSpawn {
        child,
        stdout,
        stderr,
        tree: Some(tree),
        timeout,
        _supervisor_stdin: supervisor_stdin,
        capture,
    })
}
