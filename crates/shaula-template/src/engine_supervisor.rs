//! Engine fence supervisor (R7-06 Windows / R9-09 Unix). Assigning a
//! freshly-spawned child to a Job AFTER spawn leaves a window in which
//! the engine may already run and fork un-fenced descendants; a
//! CREATE_SUSPENDED + ResumeThread fix would need raw FFI, which the
//! workspace unsafe-forbid rule prohibits. And on Unix, a bare process
//! group dies with nothing when the daemon is SIGKILLed — Drop never
//! runs. So the daemon re-executes THIS binary in a hidden supervisor
//! mode on EVERY platform instead:
//!
//! - Windows: the supervisor joins a kill-on-close job object BEFORE
//!   spawning the engine, which makes the engine — and every descendant
//!   it ever creates — a BORN member of that job. The atomic equivalent
//!   the spec requires (0001 §491).
//! - Unix: the daemon spawns the supervisor as its OWN process-group
//!   leader (`process_group(0)`); the engine inherits the group. The
//!   supervisor watches its stdin and SIGKILLs the whole group when the
//!   daemon dies — even by SIGKILL, where no Drop runs (R9-09).

use std::path::PathBuf;

use shaula_core::error::{CoreResult, ReasonCode};

use super::err;

/// Hidden argv marker that turns this binary into the engine fence
/// supervisor. INTERNAL spawn contract — never a documented CLI surface.
pub const FENCE_ARG: &str = "__shaula-engine-fence-supervisor";

/// Exit code when the supervisor could not establish the fence or spawn
/// the engine: it surfaces as an ordinary nonzero engine failure, never
/// as silent success.
const FENCE_INIT_FAILED: i32 = 3;

/// Test-hook env var: a test-harness binary cannot route the hidden
/// marker, so tests point this at the real `shaula` binary
/// (`CARGO_BIN_EXE_shaula`). Production never sets it and uses
/// `current_exe`.
const SUPERVISOR_EXE_ENV: &str = "SHAULA_ENGINE_FENCE_EXE";

/// The executable that hosts [`run_fence_supervisor`]: production uses
/// THIS binary via `current_exe`.
pub fn supervisor_executable() -> CoreResult<PathBuf> {
    if let Ok(path) = std::env::var(SUPERVISOR_EXE_ENV) {
        return Ok(PathBuf::from(path));
    }
    std::env::current_exe().map_err(|e| {
        err(
            ReasonCode::TemplateExecutionFailed,
            format!("engine fence supervisor path unavailable: {e}"),
        )
    })
}

/// The supervisor body — runs in a re-executed copy of THIS binary with
/// argv `[<engine program>, <engine args...>]` and answers with the
/// process exit code. It kills nothing directly: the fence (job on
/// Windows, process group on Unix) does, whenever this process exits for
/// ANY reason — engine exit, daemon-initiated kill, supervisor crash, or
/// daemon death.
pub fn run_fence_supervisor(argv: &[String]) -> i32 {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let Some(program) = argv.first() else {
        return FENCE_INIT_FAILED;
    };
    // Per-platform fence establishment, BEFORE the engine can run: a
    // failure here is a definite init failure, never a degraded fence.
    #[cfg(windows)]
    {
        let Ok(job) = win32job::Job::create() else {
            return FENCE_INIT_FAILED;
        };
        let mut info = win32job::ExtendedLimitInfo::new();
        info.limit_kill_on_job_close();
        // SELF assignment first: from the successful spawn below on, the
        // engine is a BORN member of the job — there is no un-fenced
        // window.
        if job.set_extended_limit_info(&info).is_err() || job.assign_process(-1).is_err() {
            return FENCE_INIT_FAILED;
        }
        // The job handle must stay open for our whole lifetime — its
        // close IS the tree kill — so it is deliberately kept: the OS
        // closes it when this process exits, firing kill-on-close.
        std::mem::forget(job);
    }

    let mut builder = std::process::Command::new(program);
    builder
        .args(&argv[1..])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());
    let mut child = match builder.spawn() {
        Ok(child) => child,
        Err(_) => return FENCE_INIT_FAILED,
    };

    // Daemon-death watchdog: the daemon holds the write end of our stdin
    // for as long as the invocation lives, so even a SIGKILLed daemon
    // EOFs us — terminate the tree and exit. `engine_done` keeps a
    // benign late EOF from clobbering the engine's real exit code.
    let engine_done = Arc::new(AtomicBool::new(false));
    {
        let engine_done = Arc::clone(&engine_done);
        std::thread::spawn(move || {
            use std::io::Read;
            let stdin = std::io::stdin();
            let mut buf = [0u8; 512];
            let mut lock = stdin.lock();
            loop {
                match lock.read(&mut buf) {
                    Ok(n) if n > 0 => {}
                    _ => break,
                }
            }
            if !engine_done.load(Ordering::SeqCst) {
                terminate_tree();
            }
        });
    }
    let code = match child.wait() {
        Ok(status) => status.code().unwrap_or(1),
        Err(_) => FENCE_INIT_FAILED,
    };
    engine_done.store(true, Ordering::SeqCst);
    code
}

/// Platform fence termination driven by the watchdog: on Unix the
/// supervisor IS the process-group leader (the daemon spawned it with
/// `process_group(0)`), so one SIGKILL to the own group takes the engine
/// and every descendant with us. On Windows exiting closes the
/// kill-on-close job, which is the same guarantee.
fn terminate_tree() -> ! {
    #[cfg(unix)]
    {
        use nix::sys::signal::{killpg, Signal};
        use nix::unistd::getpgrp;
        let _ = killpg(getpgrp(), Signal::SIGKILL);
    }
    std::process::exit(1);
}
