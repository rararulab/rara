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

//! Web frontend dev server — spawns `bun run dev` in the `web/` directory.
//!
//! Vite (via bun) handles HMR, SPA fallback, and proxying `/api` requests
//! to the backend server. The child process is killed on app shutdown via
//! `kill_on_drop` + [`CancellationToken`].

use std::path::PathBuf;

use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Spawn the Vite dev server via `bun run dev` in `web_dir`.
///
/// Returns immediately (no-op) when `web_dir/package.json` does not exist.
/// The `_port` parameter is ignored — Vite uses its own port from
/// `vite.config.ts` (default 5173).
pub async fn start_web_server(web_dir: PathBuf, _port: u16, cancel: CancellationToken) {
    if !web_dir.join("package.json").exists() {
        info!(
            path = %web_dir.display(),
            "web/package.json not found, skipping frontend server"
        );
        return;
    }

    info!(path = %web_dir.display(), "starting web frontend server (bun run dev)");

    let mut child = match Command::new("bun")
        .args(["run", "dev"])
        .current_dir(&web_dir)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => {
            info!("web frontend server started (bun run dev)");
            child
        }
        Err(e) => {
            error!(%e, "failed to start web frontend server — is bun installed?");
            return;
        }
    };

    tokio::select! {
        () = cancel.cancelled() => {
            info!("shutting down web frontend server");
            child.kill().await.ok();
        }
        status = child.wait() => {
            match status {
                Ok(s) => warn!(code = ?s.code(), "web frontend server exited unexpectedly"),
                Err(e) => error!(%e, "web frontend server error"),
            }
        }
    }
}
