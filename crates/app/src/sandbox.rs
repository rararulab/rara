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
/// at [`GUEST_WORKSPACE`] (read-write) and applies the supplied
/// [`NetworkPolicy`].
pub async fn sandbox_for_session(
    config: &SandboxToolConfig,
    sandboxes: &SandboxMap,
    session_key: SessionKey,
    network: NetworkPolicy,
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
                .network(network)
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

/// Standard "sandbox not configured" error returned by tools that require
/// a sandbox when the operator has not set `sandbox:` in YAML.
pub fn sandbox_not_configured_error(tool: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "{tool} is unavailable: `sandbox.default_rootfs_image` is not set in config.yaml. Add a \
         `sandbox:` block (see config.example.yaml) and restart."
    )
}
