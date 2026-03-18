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

//! # Panic Hook
//!
//! Enhanced panic handling with structured logging and backtraces.

use std::{panic, sync::LazyLock};

use backtrace::Backtrace;
use opentelemetry::{global, metrics::Counter};

/// OpenTelemetry counter for tracking application panics.
pub static PANIC_COUNTER: LazyLock<Counter<u64>> = LazyLock::new(|| {
    global::meter("rara-telemetry")
        .u64_counter("rara.panic.total")
        .with_description("Total application panics")
        .build()
});

/// Set up enhanced panic handling with structured logging.
///
/// Replaces the default panic handler with one that:
/// - Logs panics as structured tracing events
/// - Captures and logs backtraces
/// - Increments panic counter metrics
/// - Includes span context when available
pub fn set_panic_hook() {
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic| {
        let backtrace = Backtrace::new();
        let backtrace = format!("{backtrace:?}");
        if let Some(location) = panic.location() {
            tracing::error!(
                message = %panic,
                backtrace = %backtrace,
                panic.file = location.file(),
                panic.line = location.line(),
                panic.column = location.column(),
            );
        } else {
            tracing::error!(message = %panic, backtrace = %backtrace);
        }
        PANIC_COUNTER.add(1, &[]);
        default_hook(panic);
    }));
}
