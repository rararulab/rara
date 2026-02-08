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

//! # Panic Hook and Deadlock Detection
//!
//! Enhanced panic handling with structured logging, backtraces, and optional
//! deadlock detection. Provides better error reporting and monitoring.

#[cfg(feature = "deadlock_detection")]
use std::time::Duration;
use std::{panic, sync::LazyLock};

use backtrace::Backtrace;
use prometheus::{IntCounter, register_int_counter};

/// Prometheus counter for tracking application panics.
pub static PANIC_COUNTER: LazyLock<IntCounter> =
    LazyLock::new(|| register_int_counter!("rsketch_panic_counter", "panic_counter").unwrap());

/// Set up enhanced panic handling with structured logging.
///
/// Replaces the default panic handler with one that:
/// - Logs panics as structured tracing events
/// - Captures and logs backtraces
/// - Increments panic counter metrics
/// - Includes span context when available
/// - Optionally runs deadlock detection (if feature enabled)
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
        PANIC_COUNTER.inc();
        default_hook(panic);
    }));

    // Start deadlock detection thread if feature is enabled
    #[cfg(feature = "deadlock_detection")]
    let _ = std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(5));
            let deadlocks = parking_lot::deadlock::check_deadlock();
            if deadlocks.is_empty() {
                continue;
            }

            tracing::info!("{} deadlocks detected", deadlocks.len());
            for (i, threads) in deadlocks.iter().enumerate() {
                tracing::info!("Deadlock #{}", i);
                for t in threads {
                    tracing::info!("Thread Id {:#?}", t.thread_id());
                    tracing::info!("{:#?}", t.backtrace());
                }
            }
        }
    });
}
