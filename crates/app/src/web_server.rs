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

//! Lightweight static file server for the web frontend.
//!
//! When `web/dist/` exists (pre-built frontend), spawns a child process to
//! serve it on a separate port. Tries `npx serve` first (SPA-friendly with
//! `--single`), falls back to `python3 -m http.server`.

use std::{path::PathBuf, process::Stdio};

use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Spawn a static file server for the web frontend.
///
/// Serves `dist_dir` on the given port. Uses `npx serve` if available,
/// falls back to Python's `http.server`. The child process is killed when
/// `cancel` fires (`kill_on_drop`).
///
/// Returns immediately (no-op) when `dist_dir/index.html` does not exist.
pub async fn start_web_server(dist_dir: PathBuf, port: u16, cancel: CancellationToken) {
    if !dist_dir.join("index.html").exists() {
        info!(
            path = %dist_dir.display(),
            "web/dist not found, skipping frontend server"
        );
        return;
    }

    info!(port, path = %dist_dir.display(), "starting web frontend server");

    let dist_str = dist_dir.to_str().unwrap_or(".");

    // Try `npx serve` first — better SPA support with --single flag.
    let mut child = match Command::new("npx")
        .args([
            "serve",
            "--single",
            "--listen",
            &format!("tcp://0.0.0.0:{port}"),
            dist_str,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => {
            info!(
                port,
                "Web UI available at http://0.0.0.0:{port} (npx serve)"
            );
            child
        }
        Err(_) => {
            // Fallback: python3 -m http.server (no SPA routing).
            match Command::new("python3")
                .args([
                    "-m",
                    "http.server",
                    &port.to_string(),
                    "--directory",
                    dist_str,
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
            {
                Ok(child) => {
                    warn!(
                        port,
                        "Web UI available at http://0.0.0.0:{port} (python3 http.server, no SPA \
                         fallback)"
                    );
                    child
                }
                Err(e) => {
                    error!(
                        %e,
                        "failed to start web frontend server — neither 'npx serve' nor 'python3' available"
                    );
                    return;
                }
            }
        }
    };

    // Wait for cancellation or unexpected child exit.
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
