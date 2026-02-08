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

use std::sync::LazyLock;

use prometheus::{
    HistogramVec, IntCounterVec, IntGaugeVec, register_histogram_vec, register_int_counter_vec,
    register_int_gauge_vec,
};

pub const WORKER_LABEL: &str = "worker";

pub static WORKER_STARTED: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "worker_started_total",
        "Total number of workers started",
        &[WORKER_LABEL]
    )
    .unwrap()
});

pub static WORKER_STOPPED: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "worker_stopped_total",
        "Total number of workers stopped gracefully",
        &[WORKER_LABEL]
    )
    .unwrap()
});

pub static WORKER_ERRORS: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "worker_errors_total",
        "Total number of worker errors",
        &[WORKER_LABEL]
    )
    .unwrap()
});

pub static WORKER_START_ERRORS: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "worker_start_errors_total",
        "Total number of worker start errors",
        &[WORKER_LABEL]
    )
    .unwrap()
});

pub static WORKER_SHUTDOWN_ERRORS: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "worker_shutdown_errors_total",
        "Total number of worker shutdown errors",
        &[WORKER_LABEL]
    )
    .unwrap()
});

pub static WORKER_EXECUTIONS: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "worker_executions_total",
        "Total number of worker executions",
        &[WORKER_LABEL]
    )
    .unwrap()
});

pub static WORKER_EXECUTION_ERRORS: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "worker_execution_errors_total",
        "Total number of worker execution errors",
        &[WORKER_LABEL]
    )
    .unwrap()
});

pub static WORKER_PAUSED: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "worker_paused_total",
        "Total number of times workers were paused",
        &[WORKER_LABEL]
    )
    .unwrap()
});

pub static WORKER_RESUMED: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "worker_resumed_total",
        "Total number of times workers were resumed",
        &[WORKER_LABEL]
    )
    .unwrap()
});

pub static WORKER_ACTIVE: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    register_int_gauge_vec!(
        "worker_active",
        "Whether the worker is currently active (1) or not (0)",
        &[WORKER_LABEL]
    )
    .unwrap()
});

pub static WORKER_EXECUTION_DURATION_SECONDS: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "worker_execution_duration_seconds",
        "Worker execution duration in seconds",
        &[WORKER_LABEL]
    )
    .unwrap()
});
