// Copyright 2025 Rararulab
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

//! App-level sandbox sharing primitives.
//!
//! Holds the per-session [`SandboxMap`] type alias used by every sandbox-aware
//! tool (`run_code`, `bash`) and the `SandboxCleanupHook`. The map is built
//! once in `boot.rs` and cloned into each consumer so a single VM is reused
//! across tool invocations within a session.
//!
//! # Network policy fusion
//!
//! Every tool that shares a per-session VM (`bash`, `run_code`) also shares
//! its [`NetworkPolicy`]. The first caller in a session creates the VM, and
//! a [`NetworkPolicy`] argument that varied per-call would be silently
//! dropped on every subsequent call — a security boundary leak (see PR
//! #1946 review). To eliminate that footgun, [`sandbox_for_session`] takes
//! no network argument: instead the fused policy is computed **once** at
//! VM creation from the shared [`SandboxToolConfig`] via
//! [`fused_network_policy`].
//!
//! The fusion rule is the union (most-permissive) across all sandbox-using
//! tools that may run in the same session:
//!
//! - if **every** caller wants `Disabled`, the result is `Disabled` (the
//!   default-deny ground state — an absent or empty allow-list contributes
//!   nothing);
//! - otherwise the result is `Enabled` with the de-duplicated union of the
//!   callers' allow-lists. That union is always a concrete, host/CIDR-scoped
//!   list. No config input can produce `Enabled { allow_net: [] }` — the
//!   empty-list value that boxlite treats as *full outbound* is deliberately
//!   unreachable, because boxlite v0.9.7 has no "all hosts" token and we do not
//!   reintroduce one as a rara sentinel (see #2216).
//!
//! Both contributors are config-driven and default-deny:
//!
//! - `bash` — config at [`SandboxToolConfig::bash`] (`None` ⇒ `Disabled`, empty
//!   `allow_net` ⇒ `Disabled`, non-empty ⇒ `Enabled` with that list);
//! - `run_code` — config at [`SandboxToolConfig::run_code`], same semantics.
//!   Untrusted, LLM-generated code gets **no** egress unless the operator
//!   explicitly enumerates hosts.

use std::{future::Future, sync::Arc, time::Duration};

use dashmap::DashMap;
use rara_kernel::session::SessionKey;
use rara_sandbox::{NetworkPolicy, Sandbox, SandboxConfig, VolumeMount};
use tokio::sync::Mutex;

use crate::SandboxToolConfig;

/// Per-session sandbox lookup table.
///
/// Wrapped in `Arc` so the tools and the cleanup hook share a single map.
pub type SandboxMap = Arc<DashMap<SessionKey, Arc<Mutex<Sandbox>>>>;

/// Guest-side mount point for the host workspace directory.
///
/// All path-translating tools rewrite `<workspace>/<rest>` to
/// `/workspace/<rest>` when handing arguments to the sandbox.
pub const GUEST_WORKSPACE: &str = "/workspace";

/// Look up the existing sandbox for `session_key` or create one.
///
/// Concurrent invocations within the same session serialise on the
/// per-session mutex returned here. The created VM mounts the host workspace
/// at [`GUEST_WORKSPACE`] (read-write) and applies the **fused** network
/// policy (see [`fused_network_policy`] and the module docs).
///
/// The network policy is derived once from `config`; it is **not** a
/// per-call argument. Per-call overrides would be silently dropped on
/// every cache hit and reintroduce the first-caller-wins leak that the
/// fusion rule was added to close.
pub async fn sandbox_for_session(
    config: &SandboxToolConfig,
    sandboxes: &SandboxMap,
    session_key: SessionKey,
) -> anyhow::Result<Arc<Mutex<Sandbox>>> {
    // entry() closes the create-twice race: if two first-calls hit the same
    // shard concurrently, only one reaches Vacant and runs Sandbox::create.
    let entry = sandboxes.entry(session_key);
    let arc = match entry {
        dashmap::mapref::entry::Entry::Occupied(o) => Arc::clone(o.get()),
        dashmap::mapref::entry::Entry::Vacant(v) => {
            let workspace_mount = VolumeMount::builder()
                .host_path(rara_paths::workspace_dir().clone())
                .guest_path(GUEST_WORKSPACE.to_owned())
                .build();
            let cfg = SandboxConfig::builder()
                .rootfs_image(config.default_rootfs_image.clone())
                .volumes(vec![workspace_mount])
                .network(fused_network_policy(config))
                .working_dir(GUEST_WORKSPACE.to_owned())
                .build();
            let sandbox = Sandbox::create(cfg)
                .await
                .map_err(|e| anyhow::anyhow!("failed to create sandbox: {e}"))?;
            let arc = Arc::new(Mutex::new(sandbox));
            v.insert(Arc::clone(&arc));
            arc
        }
    };
    Ok(arc)
}

/// Compute the fused [`NetworkPolicy`] for a per-session VM by taking the
/// most-permissive policy across every sandbox-using tool that may run in
/// the session.
///
/// See the module-level "Network policy fusion" docs for the rule. Both
/// contributors — `bash` and `run_code` — are config-driven and default-deny:
/// an absent block or an empty `allow_net` contributes `Disabled`; a non-empty
/// `allow_net` contributes `Enabled` with that concrete host/CIDR list. When
/// every contributor is `Disabled` the result is `Disabled`; otherwise it is
/// `Enabled` with the de-duplicated union of the non-empty lists. No input can
/// yield `Enabled { allow_net: [] }` (boxlite's full-outbound sentinel) — that
/// footgun is deliberately unreachable (see #2216).
pub fn fused_network_policy(config: &SandboxToolConfig) -> NetworkPolicy {
    // Each contributor maps to its allow-list. An absent block or an empty
    // `allow_net` is a default-deny contributor (empty vec); a non-empty
    // `allow_net` is a scoped contributor. `run_code` is no longer a
    // hardcoded full-net floor — its egress is operator config, same as bash.
    let contributors = [
        config.run_code.as_ref().map(|c| c.allow_net.as_slice()),
        config.bash.as_ref().map(|c| c.allow_net.as_slice()),
    ];

    // Union the non-empty allow-lists, preserving first-seen order and
    // dropping duplicates. A contributor that is absent or empty adds
    // nothing, so it can only ever tighten — never widen — the union.
    let mut allow_net: Vec<String> = Vec::new();
    for host in contributors.into_iter().flatten().flatten() {
        if !allow_net.contains(host) {
            allow_net.push(host.clone());
        }
    }

    // All contributors default-deny → no network at all. Otherwise the union
    // is a concrete, host/CIDR-scoped list; it is never empty here, so the
    // `Enabled { allow_net: [] }` full-outbound sentinel is unreachable.
    if allow_net.is_empty() {
        NetworkPolicy::Disabled
    } else {
        NetworkPolicy::Enabled { allow_net }
    }
}

/// Standard "sandbox not configured" error returned by tools that require
/// a sandbox when the operator has not set `sandbox:` in YAML.
pub fn sandbox_not_configured_error(tool: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "{tool} is unavailable: `sandbox.default_rootfs_image` is not set in config.yaml. Add a \
         `sandbox:` block (see config.example.yaml) and restart."
    )
}

/// Maximum ownership-reacquisition attempts before the reaper gives up.
///
/// Mechanism-tuning constant (not YAML — see `docs/guides/anti-patterns.md`
/// "mechanism-tuning constants are Rust `const`"). Sized, together with
/// [`RECLAIM_BACKOFF`], to comfortably outlast turn-cancellation propagation:
/// when a session ends with an exec still in flight, turn cancellation drops
/// the in-flight `Arc` clone within milliseconds, so a sub-second budget is
/// ample. See [`reclaim_when_idle`] and #1866.
pub(crate) const RECLAIM_MAX_ATTEMPTS: u32 = 10;

/// Delay between ownership-reacquisition attempts. See
/// [`RECLAIM_MAX_ATTEMPTS`].
pub(crate) const RECLAIM_BACKOFF: Duration = Duration::from_millis(150);

/// Outcome of a [`reclaim_when_idle`] run.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReclaimOutcome {
    /// The last outstanding clone dropped, ownership was reacquired, and the
    /// `reclaim` callback was invoked exactly once.
    Reclaimed,
    /// The retry budget was exhausted while a clone was still outstanding;
    /// `reclaim` was never invoked. Carries the final observed strong count
    /// so the caller can escalate (warn) with the residual contention.
    Exhausted { strong_count: usize },
}

/// Reacquire sole ownership of an `Arc<Mutex<T>>` once its last outstanding
/// clone drops, then hand the owned `T` to `reclaim`.
///
/// This is the mechanism behind [`SandboxCleanupHook`](crate::tools::run_code)
/// reclaiming a per-session microVM at session end. `Sandbox::destroy`
/// consumes `self`, so the reaper must own the `Sandbox` — but when a session
/// ends with an `exec` still in flight, that exec holds a clone of the `Arc`
/// for the duration of the call. Turn cancellation drops the in-flight future
/// (and its clone) moments later, so instead of giving up on the first
/// `Arc::try_unwrap` we retry with a bounded backoff until the clone clears.
///
/// The loop is generic over the payload `T` purely so it can be unit-tested
/// with a probe type and **no boxlite dependency** (CI has no boxlite runtime).
/// It never blocks the caller's own progress guarantees: callers run it inside
/// a detached task so the lifecycle pipeline's per-hook timeout is unaffected.
///
/// Returns [`ReclaimOutcome::Reclaimed`] once `reclaim` runs, or
/// [`ReclaimOutcome::Exhausted`] if the budget runs out while a clone is still
/// live (a genuinely wedged exec — not the timing window this targets). The
/// task therefore always terminates.
pub(crate) async fn reclaim_when_idle<T, F, Fut>(
    arc: Arc<Mutex<T>>,
    max_attempts: u32,
    backoff: Duration,
    reclaim: F,
) -> ReclaimOutcome
where
    F: FnOnce(T) -> Fut,
    Fut: Future<Output = ()>,
{
    let mut arc = arc;
    for attempt in 1..=max_attempts {
        match Arc::try_unwrap(arc) {
            Ok(mutex) => {
                reclaim(mutex.into_inner()).await;
                return ReclaimOutcome::Reclaimed;
            }
            Err(returned) => {
                arc = returned;
                // Sleep between attempts, but not after the final one — there
                // is no point waiting once the budget is spent.
                if attempt < max_attempts {
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    }
    ReclaimOutcome::Exhausted {
        strong_count: Arc::strong_count(&arc),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use super::*;
    use crate::{BashSandboxConfig, RunCodeSandboxConfig};

    /// Payload for the reclamation tests — stands in for `Sandbox` so the
    /// mechanism is exercised without a boxlite dependency.
    struct Probe {
        id: u32,
    }

    /// Scenario: reclamation waits for an outstanding clone, then reclaims
    /// exactly once. Core falsifier for #1866 — the single-shot `try_unwrap`
    /// would give up here and leak.
    #[tokio::test]
    async fn reclaim_waits_for_outstanding_clone_then_reclaims_once() {
        let arc = Arc::new(Mutex::new(Probe { id: 42 }));
        let clone = Arc::clone(&arc); // simulated in-flight exec's clone
        let reclaimed: Arc<StdMutex<Vec<u32>>> = Arc::new(StdMutex::new(Vec::new()));

        // The in-flight task holds its clone across a couple of backoff
        // cycles, asserts the reaper has NOT reclaimed while contended, then
        // drops the clone so ownership can be reacquired.
        let seen = Arc::clone(&reclaimed);
        let holder = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(60)).await;
            assert!(
                seen.lock().unwrap().is_empty(),
                "reclaim must not fire while a clone is outstanding"
            );
            drop(clone);
        });

        let record = Arc::clone(&reclaimed);
        let outcome = reclaim_when_idle(
            arc,
            50,
            Duration::from_millis(10),
            move |probe: Probe| async move {
                record.lock().unwrap().push(probe.id);
            },
        )
        .await;

        holder.await.expect("holder task");
        assert_eq!(outcome, ReclaimOutcome::Reclaimed);
        assert_eq!(
            *reclaimed.lock().unwrap(),
            vec![42],
            "reclaim must fire exactly once with owned ownership of the Probe"
        );
    }

    /// Scenario: reclamation fires promptly (first attempt, no sleeping) when
    /// no clone is outstanding — the common fast path.
    #[tokio::test]
    async fn reclaim_fires_immediately_when_idle() {
        let arc = Arc::new(Mutex::new(Probe { id: 7 }));
        let reclaimed: Arc<StdMutex<Vec<u32>>> = Arc::new(StdMutex::new(Vec::new()));

        let record = Arc::clone(&reclaimed);
        let started = std::time::Instant::now();
        // A huge backoff is only reachable if the reaper retried — so a fast
        // return proves it reclaimed on the first attempt without sleeping.
        let outcome = reclaim_when_idle(
            arc,
            3,
            Duration::from_secs(30),
            move |probe: Probe| async move {
                record.lock().unwrap().push(probe.id);
            },
        )
        .await;
        let elapsed = started.elapsed();

        assert_eq!(outcome, ReclaimOutcome::Reclaimed);
        assert_eq!(*reclaimed.lock().unwrap(), vec![7]);
        assert!(
            elapsed < Duration::from_secs(1),
            "must reclaim on the first attempt without exhausting the retry budget (elapsed \
             {elapsed:?})"
        );
    }

    /// Scenario: reclamation is bounded when the outstanding clone never
    /// drops — proves the detached task always terminates and surfaces the
    /// residual rather than spinning forever or leaking silently.
    #[tokio::test]
    async fn reclaim_bounded_when_clone_never_drops() {
        let arc = Arc::new(Mutex::new(Probe { id: 1 }));
        let clone = Arc::clone(&arc); // held for the entire run, never dropped
        let reclaimed: Arc<StdMutex<Vec<u32>>> = Arc::new(StdMutex::new(Vec::new()));

        let record = Arc::clone(&reclaimed);
        let outcome = reclaim_when_idle(
            arc,
            3,
            Duration::from_millis(5),
            move |probe: Probe| async move {
                record.lock().unwrap().push(probe.id);
            },
        )
        .await;

        // Bounded termination with the exhaustion surfaced (the caller warns
        // on this variant), not a silent swallow or a forever-spin.
        assert_eq!(outcome, ReclaimOutcome::Exhausted { strong_count: 2 });
        assert!(
            reclaimed.lock().unwrap().is_empty(),
            "reclaim must never fire while the clone is still held"
        );
        drop(clone);
    }

    fn cfg(
        run_code: Option<RunCodeSandboxConfig>,
        bash: Option<BashSandboxConfig>,
    ) -> SandboxToolConfig {
        SandboxToolConfig::builder()
            .default_rootfs_image("alpine:latest".to_owned())
            .maybe_run_code(run_code)
            .maybe_bash(bash)
            .build()
    }

    fn run_code(allow_net: &[&str]) -> RunCodeSandboxConfig {
        RunCodeSandboxConfig::builder()
            .allow_net(allow_net.iter().map(|s| (*s).to_owned()).collect())
            .build()
    }

    fn bash(allow_net: &[&str]) -> BashSandboxConfig {
        BashSandboxConfig::builder()
            .allow_net(allow_net.iter().map(|s| (*s).to_owned()).collect())
            .build()
    }

    /// Default-deny: no `bash`, no `run_code` → the fused policy is
    /// `Disabled`. This inverts the pre-#2216 behavior, where the same input
    /// returned `Enabled { allow_net: [] }` (full outbound) because
    /// `run_code` was a hardcoded full-net floor.
    #[test]
    fn no_config_yields_disabled_network() {
        assert!(matches!(
            fused_network_policy(&cfg(None, None)),
            NetworkPolicy::Disabled
        ));
    }

    /// An empty `run_code.allow_net` is a default-deny contributor, not full
    /// outbound — with bash also absent, the fused policy is `Disabled`.
    #[test]
    fn empty_run_code_allowlist_is_disabled() {
        assert!(matches!(
            fused_network_policy(&cfg(Some(run_code(&[])), None)),
            NetworkPolicy::Disabled
        ));
    }

    /// The operator's explicit `run_code` allow-list is honored verbatim.
    #[test]
    fn run_code_allowlist_is_honored() {
        let policy = fused_network_policy(&cfg(
            Some(run_code(&["pypi.org", "files.pythonhosted.org"])),
            None,
        ));
        match policy {
            NetworkPolicy::Enabled { allow_net } => {
                assert_eq!(allow_net.len(), 2, "no duplicates expected");
                assert!(allow_net.contains(&"pypi.org".to_owned()));
                assert!(allow_net.contains(&"files.pythonhosted.org".to_owned()));
            }
            NetworkPolicy::Disabled => panic!("expected Enabled with the operator allow-list"),
        }
    }

    /// `bash` and `run_code` allow-lists union into one policy: both hosts
    /// present, neither dropped.
    #[test]
    fn bash_and_run_code_allowlists_union() {
        let policy = fused_network_policy(&cfg(
            Some(run_code(&["pypi.org"])),
            Some(bash(&["github.com"])),
        ));
        match policy {
            NetworkPolicy::Enabled { allow_net } => {
                assert_eq!(allow_net.len(), 2, "deduplicated union of both lists");
                assert!(allow_net.contains(&"pypi.org".to_owned()));
                assert!(allow_net.contains(&"github.com".to_owned()));
            }
            NetworkPolicy::Disabled => panic!("expected Enabled union"),
        }
    }

    /// No config input can reach the `Enabled { allow_net: [] }` full-outbound
    /// sentinel. A bare `"*"` and a `"0.0.0.0/0"` entry are passed through as
    /// ordinary boxlite patterns — each yields a *scoped* `Enabled` carrying
    /// exactly the operator's entries, never an empty list.
    #[test]
    fn no_config_input_yields_empty_allowlist_enabled() {
        for entry in ["*", "0.0.0.0/0"] {
            match fused_network_policy(&cfg(Some(run_code(&[entry])), None)) {
                NetworkPolicy::Enabled { allow_net } => {
                    assert_eq!(
                        allow_net,
                        vec![entry.to_owned()],
                        "entry must pass through verbatim, never expand to all-hosts"
                    );
                    assert!(
                        !allow_net.is_empty(),
                        "the empty-list full-outbound sentinel must be unreachable"
                    );
                }
                NetworkPolicy::Disabled => panic!("a non-empty allow-list must stay Enabled"),
            }
        }
    }
}
