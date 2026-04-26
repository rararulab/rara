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

//! Continuous CPU profiling via [Pyroscope](https://github.com/grafana/pyroscope).
//!
//! Wraps `pyroscope` + `pyroscope_pprofrs` so the `rara-cli` bootstrap can
//! turn profiling on/off from YAML config without dragging the underlying
//! crates into every binary.
//!
//! ## Cardinality contract
//!
//! Tags attached to profiles are **process-level only** — `env`, `host`,
//! `build_commit`. Per-request labels (`session_id`, `skill_name`,
//! `user_id`, …) MUST NOT be added: they explode label cardinality on
//! the Pyroscope server and turn flamegraph queries into table scans.
//!
//! ## Limitation
//!
//! `pprof-rs` samples OS-thread CPU only. It does not see async `.await`
//! stalls or tokio mutex contention — for those, use `tokio-console`
//! (separate feature flag, tracked as a future chore).

use pyroscope::{PyroscopeAgent, pyroscope::PyroscopeAgentRunning};
use pyroscope_pprofrs::{PprofConfig, pprof_backend};
use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Snafu};

/// Configuration for the Pyroscope profiling agent.
///
/// All fields are required (no Rust defaults) — the entire section is
/// optional in YAML, so omission is the "off" signal.
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
#[builder(on(String, into))]
pub struct PyroscopeConfig {
    /// Master switch. When `false`, no agent is constructed and no
    /// profiling thread is spawned — zero overhead.
    pub enabled:          bool,
    /// Pyroscope server endpoint (e.g. `http://10.0.0.183:4040`).
    pub endpoint:         String,
    /// Application name reported to Pyroscope (e.g. `"rara"`).
    pub application_name: String,
    /// CPU sampling rate in Hz (typical: 100).
    pub sample_rate:      u32,
}

/// Errors raised while wiring up the Pyroscope agent.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum ProfilingError {
    #[snafu(display("failed to build Pyroscope agent: {source}"))]
    BuildAgent { source: pyroscope::PyroscopeError },

    #[snafu(display("failed to start Pyroscope agent: {source}"))]
    StartAgent { source: pyroscope::PyroscopeError },
}

/// RAII guard that owns the running Pyroscope agent.
///
/// Drop performs a best-effort graceful shutdown: `stop()` flushes the
/// last batch, then `shutdown()` joins the worker thread. Errors during
/// drop are logged, never panicked, so a crashing collector cannot mask
/// the original program exit code.
pub struct ProfilingGuard {
    agent: Option<PyroscopeAgent<PyroscopeAgentRunning>>,
}

impl ProfilingGuard {
    /// Stop and shut down the agent explicitly. Idempotent — calling twice
    /// (or letting `Drop` finish the job) is safe.
    pub fn shutdown(mut self) { self.shutdown_inner(); }

    fn shutdown_inner(&mut self) {
        let Some(agent) = self.agent.take() else {
            return;
        };
        match agent.stop() {
            Ok(ready) => {
                ready.shutdown();
                tracing::info!("Pyroscope agent stopped");
            }
            Err(err) => {
                tracing::warn!(error = %err, "Pyroscope agent stop failed");
            }
        }
    }
}

impl Drop for ProfilingGuard {
    fn drop(&mut self) { self.shutdown_inner(); }
}

/// Initialise the Pyroscope agent if `cfg.enabled` is true.
///
/// `build_commit` is wired in by the caller from a build-time mechanism
/// (see `rara-cli`'s `shadow_rs` integration). It is treated as an
/// opaque process-lifetime tag, never per-request.
///
/// Returns:
/// - `Ok(Some(guard))` when the agent started — keep the guard alive for the
///   lifetime of the process.
/// - `Ok(None)` when `enabled = false` — no agent, no thread, zero cost.
/// - `Err(_)` when the agent could not be built/started.
pub fn init_pyroscope(
    cfg: &PyroscopeConfig,
    env: &str,
    host: &str,
    build_commit: &str,
) -> Result<Option<ProfilingGuard>, ProfilingError> {
    if !cfg.enabled {
        return Ok(None);
    }

    let pprof = PprofConfig::new().sample_rate(cfg.sample_rate);
    // `tags` is a Vec<(&str, &str)>, so build owned strings up-front and
    // borrow into the call. Pyroscope copies them internally.
    let tags: Vec<(&str, &str)> =
        vec![("env", env), ("host", host), ("build_commit", build_commit)];

    let agent = PyroscopeAgent::builder(&cfg.endpoint, &cfg.application_name)
        .backend(pprof_backend(pprof))
        .tags(tags)
        .build()
        .context(BuildAgentSnafu)?;

    let running = agent.start().context(StartAgentSnafu)?;
    tracing::info!(
        endpoint = %cfg.endpoint,
        application = %cfg.application_name,
        sample_rate = cfg.sample_rate,
        env, host, build_commit,
        "Pyroscope continuous profiling started"
    );

    Ok(Some(ProfilingGuard {
        agent: Some(running),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_returns_none_without_contacting_endpoint() {
        // Sanity check the zero-overhead contract: with `enabled = false`
        // we must short-circuit before touching the (unreachable) endpoint.
        let cfg = PyroscopeConfig::builder()
            .enabled(false)
            .endpoint("http://127.0.0.1:1") // unreachable on purpose
            .application_name("rara-test")
            .sample_rate(100_u32)
            .build();
        let guard =
            init_pyroscope(&cfg, "test", "host", "deadbeef").expect("disabled path is infallible");
        assert!(guard.is_none(), "no guard means no agent, no thread");
    }
}
