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
//! - if **every** caller wants `Disabled`, the result is `Disabled`;
//! - otherwise the result is `Enabled` with the union of allow-lists. An empty
//!   allow-list under `Enabled` means full outbound (boxlite's own default), so
//!   a single full-net caller correctly dominates.
//!
//! Today the contributors are:
//!
//! - `bash` — config at [`SandboxToolConfig::bash`] (`None` ⇒ `Disabled`, empty
//!   `allow_net` ⇒ `Disabled`, non-empty ⇒ `Enabled` with that list);
//! - `run_code` — historical full network access, modelled as `Enabled {
//!   allow_net: [] }`.

use std::sync::Arc;

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
/// See the module-level "Network policy fusion" docs for the rule. Today
/// the contributors are `bash` (config-driven) and `run_code` (historical
/// full network).
pub fn fused_network_policy(config: &SandboxToolConfig) -> NetworkPolicy {
    // run_code's contribution: historical full outbound access, modelled as
    // Enabled with an empty allow-list. This always pushes the union to
    // Enabled — but we still compute the union explicitly so that adding a
    // future, more restrictive caller (e.g. a CIDR-pinned tool) won't
    // silently widen its access.
    let run_code_enabled = true;
    let run_code_allow: Vec<String> = Vec::new();

    // bash's contribution.
    let (bash_enabled, bash_allow): (bool, Vec<String>) = match config.bash.as_ref() {
        // No `bash:` block → Disabled, empty allow-list.
        None => (false, Vec::new()),
        Some(b) if b.allow_net.is_empty() => (false, Vec::new()),
        Some(b) => (true, b.allow_net.clone()),
    };

    if !run_code_enabled && !bash_enabled {
        return NetworkPolicy::Disabled;
    }

    // Union the allow-lists. If any contributor wants full outbound
    // (Enabled with an empty list), the union must also be full outbound,
    // so we collapse the result to an empty allow-list in that case.
    let any_unrestricted =
        (run_code_enabled && run_code_allow.is_empty()) || (bash_enabled && bash_allow.is_empty());
    let allow_net = if any_unrestricted {
        Vec::new()
    } else {
        let mut merged: Vec<String> = run_code_allow;
        for host in bash_allow {
            if !merged.contains(&host) {
                merged.push(host);
            }
        }
        merged
    };
    NetworkPolicy::Enabled { allow_net }
}

/// Standard "sandbox not configured" error returned by tools that require
/// a sandbox when the operator has not set `sandbox:` in YAML.
pub fn sandbox_not_configured_error(tool: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "{tool} is unavailable: `sandbox.default_rootfs_image` is not set in config.yaml. Add a \
         `sandbox:` block (see config.example.yaml) and restart."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BashSandboxConfig;

    fn cfg(bash: Option<BashSandboxConfig>) -> SandboxToolConfig {
        SandboxToolConfig::builder()
            .default_rootfs_image("alpine:latest".to_owned())
            .maybe_bash(bash)
            .build()
    }

    /// Even when bash is `None` (Disabled), `run_code` keeps full outbound
    /// access — so the fused policy is `Enabled { allow_net: [] }`.
    #[test]
    fn fuses_run_code_full_net_with_disabled_bash() {
        match fused_network_policy(&cfg(None)) {
            NetworkPolicy::Enabled { allow_net } => assert!(allow_net.is_empty()),
            NetworkPolicy::Disabled => panic!("run_code should keep network up"),
        }
    }

    /// A bash allow-list does NOT shrink `run_code`'s full outbound access:
    /// the union with an unrestricted caller is unrestricted.
    #[test]
    fn fuses_run_code_full_net_dominates_bash_allowlist() {
        let bash = BashSandboxConfig::builder()
            .allow_net(vec!["github.com".to_owned()])
            .build();
        match fused_network_policy(&cfg(Some(bash))) {
            NetworkPolicy::Enabled { allow_net } => assert!(allow_net.is_empty()),
            NetworkPolicy::Disabled => panic!("expected Enabled"),
        }
    }

    /// Empty `allow_net` on bash collapses to Disabled for that caller, but
    /// `run_code` still pushes the union to Enabled (full outbound).
    #[test]
    fn fuses_empty_bash_allowlist_as_disabled_caller() {
        let bash = BashSandboxConfig::builder().allow_net(vec![]).build();
        match fused_network_policy(&cfg(Some(bash))) {
            NetworkPolicy::Enabled { allow_net } => assert!(allow_net.is_empty()),
            NetworkPolicy::Disabled => panic!("run_code should keep network up"),
        }
    }
}
