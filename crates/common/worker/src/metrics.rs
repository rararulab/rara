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

//! OpenTelemetry metrics for the worker subsystem.

use std::sync::LazyLock;

use opentelemetry::{
    global,
    metrics::{Counter, Histogram, UpDownCounter},
};

/// Return the shared meter scoped to the worker crate.
fn meter() -> opentelemetry::metrics::Meter { global::meter("rara-worker") }

// -- Worker lifecycle --------------------------------------------------------

/// Total number of workers started.
pub static WORKER_STARTED: LazyLock<Counter<u64>> = LazyLock::new(|| {
    meter()
        .u64_counter("worker.started")
        .with_description("Total number of workers started")
        .build()
});

/// Total number of workers stopped gracefully.
pub static WORKER_STOPPED: LazyLock<Counter<u64>> = LazyLock::new(|| {
    meter()
        .u64_counter("worker.stopped")
        .with_description("Total number of workers stopped gracefully")
        .build()
});

/// Whether the worker is currently active (gauge via up-down counter).
pub static WORKER_ACTIVE: LazyLock<UpDownCounter<i64>> = LazyLock::new(|| {
    meter()
        .i64_up_down_counter("worker.active")
        .with_description("Whether the worker is currently active")
        .build()
});

// -- Worker errors -----------------------------------------------------------

/// Total number of worker errors.
pub static WORKER_ERRORS: LazyLock<Counter<u64>> = LazyLock::new(|| {
    meter()
        .u64_counter("worker.errors")
        .with_description("Total number of worker errors")
        .build()
});

/// Total number of worker start errors.
pub static WORKER_START_ERRORS: LazyLock<Counter<u64>> = LazyLock::new(|| {
    meter()
        .u64_counter("worker.start_errors")
        .with_description("Total number of worker start errors")
        .build()
});

/// Total number of worker shutdown errors.
pub static WORKER_SHUTDOWN_ERRORS: LazyLock<Counter<u64>> = LazyLock::new(|| {
    meter()
        .u64_counter("worker.shutdown_errors")
        .with_description("Total number of worker shutdown errors")
        .build()
});

// -- Worker execution --------------------------------------------------------

/// Total number of worker executions.
pub static WORKER_EXECUTIONS: LazyLock<Counter<u64>> = LazyLock::new(|| {
    meter()
        .u64_counter("worker.executions")
        .with_description("Total number of worker executions")
        .build()
});

/// Total number of worker execution errors.
pub static WORKER_EXECUTION_ERRORS: LazyLock<Counter<u64>> = LazyLock::new(|| {
    meter()
        .u64_counter("worker.execution_errors")
        .with_description("Total number of worker execution errors")
        .build()
});

/// Worker execution duration in seconds.
pub static WORKER_EXECUTION_DURATION_SECONDS: LazyLock<Histogram<f64>> = LazyLock::new(|| {
    meter()
        .f64_histogram("worker.execution.duration")
        .with_description("Worker execution duration in seconds")
        .with_unit("s")
        .build()
});

// -- Worker state transitions ------------------------------------------------

/// Total number of times workers were paused.
pub static WORKER_PAUSED: LazyLock<Counter<u64>> = LazyLock::new(|| {
    meter()
        .u64_counter("worker.paused")
        .with_description("Total number of times workers were paused")
        .build()
});

/// Total number of times workers were resumed.
pub static WORKER_RESUMED: LazyLock<Counter<u64>> = LazyLock::new(|| {
    meter()
        .u64_counter("worker.resumed")
        .with_description("Total number of times workers were resumed")
        .build()
});
