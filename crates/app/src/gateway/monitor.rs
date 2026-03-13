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

//! Process resource monitor for the rara server child process.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessesToUpdate, System};
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// Snapshot — the metrics we collect each tick
// ---------------------------------------------------------------------------

/// Point-in-time resource snapshot for the monitored child process.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ProcessSnapshot {
    /// Process ID being monitored.
    pub pid: Option<u32>,
    /// CPU usage percentage (0.0–100.0+, can exceed 100% on multi-core).
    pub cpu_percent: f32,
    /// Resident set size in bytes.
    pub rss_bytes: u64,
    /// Virtual memory size in bytes.
    pub virt_bytes: u64,
    /// Number of threads (Linux only, 0 elsewhere).
    pub thread_count: u64,
    /// Open file descriptors (Linux only via /proc, 0 elsewhere).
    pub open_fds: u64,
    /// Total bytes read from disk since process start.
    pub disk_read_bytes: u64,
    /// Total bytes written to disk since process start.
    pub disk_write_bytes: u64,
    /// Process uptime in seconds.
    pub uptime_secs: u64,
    /// Timestamp of this snapshot (RFC 3339).
    pub sampled_at: String,
}

// ---------------------------------------------------------------------------
// Alert thresholds — dynamically configurable via Telegram
// ---------------------------------------------------------------------------

/// Thresholds for resource alerts. `None` = alert disabled.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AlertThresholds {
    /// CPU usage percentage threshold.
    pub cpu_percent: Option<f32>,
    /// RSS memory threshold in megabytes.
    pub mem_mb: Option<u64>,
}

impl AlertThresholds {
    /// Return a human-readable summary.
    pub fn summary(&self) -> String {
        let cpu = self
            .cpu_percent
            .map(|v| format!("{v}%"))
            .unwrap_or_else(|| "off".into());
        let mem = self
            .mem_mb
            .map(|v| format!("{v} MB"))
            .unwrap_or_else(|| "off".into());
        format!("cpu: {cpu}, mem: {mem}")
    }
}

/// Shared handle to the latest process snapshot.
pub type SnapshotHandle = Arc<RwLock<ProcessSnapshot>>;

/// Shared handle to alert thresholds.
pub type ThresholdsHandle = Arc<RwLock<AlertThresholds>>;

// ---------------------------------------------------------------------------
// GatewayState — persisted runtime state
// ---------------------------------------------------------------------------

/// Persisted gateway runtime state (survives restarts).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GatewayState {
    #[serde(default)]
    pub alert_thresholds: AlertThresholds,
}

/// Path to the gateway runtime state file.
fn state_file_path() -> std::path::PathBuf {
    rara_paths::config_dir().join("gateway-state.yaml")
}

/// Load gateway state from disk. Returns default if file doesn't exist or is invalid.
pub fn load_gateway_state() -> GatewayState {
    let path = state_file_path();
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_yaml::from_str(&content).unwrap_or_default(),
        Err(_) => GatewayState::default(),
    }
}

/// Save gateway state to disk. Errors are logged but not propagated.
pub fn save_gateway_state(state: &GatewayState) {
    let path = state_file_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_yaml::to_string(state) {
        Ok(yaml) => {
            if let Err(e) = std::fs::write(&path, yaml) {
                tracing::warn!(error = %e, "Failed to save gateway state");
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to serialize gateway state");
        }
    }
}

// ---------------------------------------------------------------------------
// ProcessMonitor — collects metrics each tick
// ---------------------------------------------------------------------------

/// Collects per-PID process metrics using `sysinfo`.
pub struct ProcessMonitor {
    sys: System,
    snapshot: SnapshotHandle,
    thresholds: ThresholdsHandle,
    last_alert_at: Option<std::time::Instant>,
}

impl ProcessMonitor {
    /// Create a new monitor with shared handles.
    pub fn new(snapshot: SnapshotHandle, thresholds: ThresholdsHandle) -> Self {
        Self {
            sys: System::new(),
            snapshot,
            thresholds,
            last_alert_at: None,
        }
    }

    /// Refresh metrics for the given PID. Returns a list of breached alerts.
    pub async fn tick(&mut self, pid: Option<u32>) -> Vec<String> {
        let snap = self.collect(pid);
        let raw_alerts = self.check_thresholds(&snap).await;
        *self.snapshot.write().await = snap;

        // Cooldown: suppress alerts if less than 60s since last alert.
        if raw_alerts.is_empty() {
            return vec![];
        }
        if let Some(last) = self.last_alert_at {
            if last.elapsed() < std::time::Duration::from_secs(60) {
                return vec![];
            }
        }
        self.last_alert_at = Some(std::time::Instant::now());
        raw_alerts
    }

    fn collect(&mut self, pid: Option<u32>) -> ProcessSnapshot {
        let Some(raw_pid) = pid else {
            return ProcessSnapshot {
                sampled_at: chrono::Local::now().to_rfc3339(),
                ..Default::default()
            };
        };

        let sys_pid = Pid::from_u32(raw_pid);
        self.sys
            .refresh_processes(ProcessesToUpdate::Some(&[sys_pid]), true);

        let Some(proc_info) = self.sys.process(sys_pid) else {
            return ProcessSnapshot {
                pid: Some(raw_pid),
                sampled_at: chrono::Local::now().to_rfc3339(),
                ..Default::default()
            };
        };

        let disk = proc_info.disk_usage();

        // Thread count: tasks() only available on Linux.
        let thread_count = proc_info
            .tasks()
            .map(|t| t.len() as u64)
            .unwrap_or(0);

        // Open FDs: read from /proc on Linux, unavailable elsewhere.
        let open_fds = read_fd_count(raw_pid);

        ProcessSnapshot {
            pid: Some(raw_pid),
            cpu_percent: proc_info.cpu_usage(),
            rss_bytes: proc_info.memory(),
            virt_bytes: proc_info.virtual_memory(),
            thread_count,
            open_fds,
            disk_read_bytes: disk.read_bytes,
            disk_write_bytes: disk.written_bytes,
            uptime_secs: proc_info.run_time(),
            sampled_at: chrono::Local::now().to_rfc3339(),
        }
    }

    async fn check_thresholds(&self, snap: &ProcessSnapshot) -> Vec<String> {
        let thresholds = self.thresholds.read().await;
        let mut alerts = Vec::new();

        if let Some(cpu_limit) = thresholds.cpu_percent {
            if snap.cpu_percent > cpu_limit {
                alerts.push(format!(
                    "CPU {:.1}% > threshold {:.1}%",
                    snap.cpu_percent, cpu_limit
                ));
            }
        }

        if let Some(mem_limit) = thresholds.mem_mb {
            let rss_mb = snap.rss_bytes / (1024 * 1024);
            if rss_mb > mem_limit {
                alerts.push(format!("RSS {} MB > threshold {} MB", rss_mb, mem_limit));
            }
        }

        alerts
    }
}

/// Read open file descriptor count. Linux-only via /proc; returns 0 elsewhere.
fn read_fd_count(pid: u32) -> u64 {
    #[cfg(target_os = "linux")]
    {
        let path = format!("/proc/{pid}/fd");
        std::fs::read_dir(path)
            .map(|rd| rd.count() as u64)
            .unwrap_or(0)
    }
    #[cfg(not(target_os = "linux"))]
    {
        // macOS: lsof -p is too expensive per tick; use proc_pidinfo if needed later.
        let _ = pid;
        0
    }
}
