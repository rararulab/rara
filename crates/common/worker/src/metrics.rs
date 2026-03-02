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

use lazy_static::lazy_static;
use prometheus::*;

/// Worker name label.
pub const WORKER_LABEL: &str = "worker";

// -- Worker lifecycle --------------------------------------------------------

lazy_static! {
    /// Total number of workers started.
    pub static ref WORKER_STARTED: IntCounterVec =
        register_int_counter_vec!(
            "worker_started_total",
            "Total number of workers started",
            &[WORKER_LABEL]
        ).unwrap();

    /// Total number of workers stopped gracefully.
    pub static ref WORKER_STOPPED: IntCounterVec =
        register_int_counter_vec!(
            "worker_stopped_total",
            "Total number of workers stopped gracefully",
            &[WORKER_LABEL]
        ).unwrap();

    /// Whether the worker is currently active (1) or not (0).
    pub static ref WORKER_ACTIVE: IntGaugeVec =
        register_int_gauge_vec!(
            "worker_active",
            "Whether the worker is currently active (1) or not (0)",
            &[WORKER_LABEL]
        ).unwrap();
}

// -- Worker errors -----------------------------------------------------------

lazy_static! {
    /// Total number of worker errors.
    pub static ref WORKER_ERRORS: IntCounterVec =
        register_int_counter_vec!(
            "worker_errors_total",
            "Total number of worker errors",
            &[WORKER_LABEL]
        ).unwrap();

    /// Total number of worker start errors.
    pub static ref WORKER_START_ERRORS: IntCounterVec =
        register_int_counter_vec!(
            "worker_start_errors_total",
            "Total number of worker start errors",
            &[WORKER_LABEL]
        ).unwrap();

    /// Total number of worker shutdown errors.
    pub static ref WORKER_SHUTDOWN_ERRORS: IntCounterVec =
        register_int_counter_vec!(
            "worker_shutdown_errors_total",
            "Total number of worker shutdown errors",
            &[WORKER_LABEL]
        ).unwrap();
}

// -- Worker execution --------------------------------------------------------

lazy_static! {
    /// Total number of worker executions.
    pub static ref WORKER_EXECUTIONS: IntCounterVec =
        register_int_counter_vec!(
            "worker_executions_total",
            "Total number of worker executions",
            &[WORKER_LABEL]
        ).unwrap();

    /// Total number of worker execution errors.
    pub static ref WORKER_EXECUTION_ERRORS: IntCounterVec =
        register_int_counter_vec!(
            "worker_execution_errors_total",
            "Total number of worker execution errors",
            &[WORKER_LABEL]
        ).unwrap();

    /// Worker execution duration in seconds.
    pub static ref WORKER_EXECUTION_DURATION_SECONDS: HistogramVec =
        register_histogram_vec!(
            "worker_execution_duration_seconds",
            "Worker execution duration in seconds",
            &[WORKER_LABEL]
        ).unwrap();
}

// -- Worker state transitions ------------------------------------------------

lazy_static! {
    /// Total number of times workers were paused.
    pub static ref WORKER_PAUSED: IntCounterVec =
        register_int_counter_vec!(
            "worker_paused_total",
            "Total number of times workers were paused",
            &[WORKER_LABEL]
        ).unwrap();

    /// Total number of times workers were resumed.
    pub static ref WORKER_RESUMED: IntCounterVec =
        register_int_counter_vec!(
            "worker_resumed_total",
            "Total number of times workers were resumed",
            &[WORKER_LABEL]
        ).unwrap();
}
