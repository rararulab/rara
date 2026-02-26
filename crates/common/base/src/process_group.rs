// Copyright 2025 Crrow
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#[cfg(unix)]
use std::io;

/// Convert a raw `u32` process group ID into a [`rustix::process::Pid`],
/// rejecting PGID 0 (which means "my own process group" in POSIX).
#[cfg(unix)]
fn parse_pgid(process_group_id: u32) -> io::Result<rustix::process::Pid> {
    rustix::process::Pid::from_raw(process_group_id as i32).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "process group ID must be non-zero",
        )
    })
}

/// Send a signal to every process in the given process group.
///
/// Returns `Ok(true)` when the signal was delivered and `Ok(false)` when
/// the group no longer exists (`ESRCH` — the caller's intent is already
/// satisfied).
#[cfg(unix)]
fn signal_process_group(
    process_group_id: u32,
    signal: rustix::process::Signal,
) -> io::Result<bool> {
    let pid = parse_pgid(process_group_id)?;

    // `rustix::process::kill_process_group` wraps `kill(-pgid, sig)` through
    // rustix's safe FFI layer — no `unsafe` needed on our side.
    match rustix::process::kill_process_group(pid, signal) {
        Ok(()) => Ok(true),
        // ESRCH: no such process group — already gone, goal achieved.
        Err(e) if e == rustix::io::Errno::SRCH => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// Send `SIGTERM` to every process in the given process group (best-effort).
///
/// This is the standard **graceful-shutdown** signal on Unix: it asks each
/// process in the group to clean up and exit, but does **not** force
/// termination (use [`kill_process_group`] for that).
///
/// # Arguments
///
/// * `process_group_id` — The POSIX process group ID (PGID). Typically obtained
///   from [`std::process::Command`] when spawning a child with
///   `.process_group(0)`, which makes the child the leader of a new group whose
///   PGID equals the child's PID.
///
/// # Returns
///
/// * `Ok(true)`  — `SIGTERM` was successfully delivered to the process group.
/// * `Ok(false)` — The process group no longer exists (`ESRCH`). This is not
///   treated as an error because the caller's intent (stop the group) is
///   already satisfied.
/// * `Err(_)`    — An OS-level error occurred, e.g. `EPERM` (insufficient
///   permissions).
///
/// # Errors
///
/// Returns [`io::ErrorKind::InvalidInput`] if `process_group_id` is `0`,
/// since PGID 0 refers to the caller's own process group and sending
/// `SIGTERM` to it would kill the current process — almost certainly not
/// what the caller intended.
#[cfg(unix)]
pub fn terminate_process_group(process_group_id: u32) -> io::Result<bool> {
    signal_process_group(process_group_id, rustix::process::Signal::TERM)
}

/// Send `SIGKILL` to every process in the given process group (best-effort).
///
/// This is the **forceful-shutdown** escalation: the kernel terminates each
/// process immediately with no chance to catch or ignore the signal. Use this
/// as a last resort after [`terminate_process_group`] (`SIGTERM`) has been
/// given a grace period to take effect.
///
/// # Typical usage
///
/// ```text
/// terminate_process_group(pgid)?;       // polite: SIGTERM
/// std::thread::sleep(GRACE_PERIOD);     // wait for cleanup
/// kill_process_group(pgid)?;            // forceful: SIGKILL
/// ```
///
/// # Returns / Errors
///
/// Same semantics as [`terminate_process_group`] — see its documentation.
#[cfg(unix)]
pub fn kill_process_group(process_group_id: u32) -> io::Result<bool> {
    signal_process_group(process_group_id, rustix::process::Signal::KILL)
}

// ── Non-Unix stubs ──────────────────────────────────────────────────────

#[cfg(not(unix))]
/// No-op on non-Unix platforms.
pub fn terminate_process_group(_process_group_id: u32) -> std::io::Result<bool> { Ok(false) }

#[cfg(not(unix))]
/// No-op on non-Unix platforms.
pub fn kill_process_group(_process_group_id: u32) -> std::io::Result<bool> { Ok(false) }

// ── ProcessGroupGuard ───────────────────────────────────────────────────

/// RAII guard that terminates a process group on drop.
///
/// When dropped, the guard performs a two-phase shutdown:
///
/// 1. **`SIGTERM`** — politely asks every process in the group to exit.
/// 2. **`SIGKILL`** (after a grace period) — forcefully kills any survivors.
///
/// The grace period between SIGTERM and SIGKILL is controlled by
/// `GRACE_PERIOD` (default: 2 seconds). The SIGKILL escalation runs
/// on a detached background thread so that `Drop` itself returns immediately.
///
/// On non-Unix platforms the guard is a no-op — it stores nothing and its
/// `Drop` impl does nothing.
///
/// # Example
///
/// ```text
/// use std::process::Command;
///
/// let child = Command::new("my-server")
///     .process_group(0)   // new process group, PGID = child PID
///     .spawn()?;
///
/// let guard = ProcessGroupGuard::new(child.id());
/// // ... use the child ...
/// drop(guard);  // SIGTERM → 2s → SIGKILL
/// ```
#[cfg(unix)]
pub struct ProcessGroupGuard {
    process_group_id: u32,
}

#[cfg(not(unix))]
pub struct ProcessGroupGuard;

impl ProcessGroupGuard {
    /// Default grace period between `SIGTERM` and `SIGKILL` escalation.
    const GRACE_PERIOD: std::time::Duration = std::time::Duration::from_secs(2);

    /// Create a new guard for the given process group ID.
    ///
    /// The guard takes ownership of the lifecycle: when it is dropped, the
    /// entire process group will be terminated.
    pub fn new(process_group_id: u32) -> Self {
        #[cfg(unix)]
        {
            Self { process_group_id }
        }
        #[cfg(not(unix))]
        {
            let _ = process_group_id;
            Self
        }
    }

    /// Perform the two-phase shutdown: SIGTERM now, SIGKILL after grace period.
    #[cfg(unix)]
    fn graceful_shutdown(&self) {
        let process_group_id = self.process_group_id;

        // Phase 1: SIGTERM — ask processes to exit gracefully.
        let should_escalate = match terminate_process_group(process_group_id) {
            Ok(exists) => exists,
            Err(error) => {
                tracing::warn!(
                    process_group_id,
                    %error,
                    "failed to send SIGTERM to process group",
                );
                false
            }
        };

        if !should_escalate {
            return;
        }

        // Phase 2: SIGKILL — after a grace period, forcefully kill survivors.
        // Runs on a detached thread so Drop returns immediately.
        let grace = Self::GRACE_PERIOD;
        std::thread::spawn(move || {
            std::thread::sleep(grace);
            if let Err(error) = kill_process_group(process_group_id) {
                tracing::warn!(
                    process_group_id,
                    %error,
                    "failed to send SIGKILL to process group",
                );
            }
        });
    }

    #[cfg(not(unix))]
    fn graceful_shutdown(&self) {}
}

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) { self.graceful_shutdown(); }
}
