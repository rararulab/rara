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

use std::sync::atomic::{AtomicUsize, Ordering};

use snafu::ResultExt;
use tokio::runtime::{Builder as TokioBuilder, Runtime};

use crate::{
    error::{self, Result},
    options::{RuntimeOptions, cpu_threads},
};

impl RuntimeOptions {
    /// Build a multi-thread Tokio runtime with optional IO/time drivers and
    /// sequential thread names. Defaults worker threads to CPU count when
    /// unspecified and prefixes threads using `thread_name`.
    ///
    /// # Errors
    /// Returns an error if the runtime fails to build.
    pub fn create(self) -> Result<Runtime> {
        let mut builder = TokioBuilder::new_multi_thread();

        let worker_threads = self.worker_threads.unwrap_or_else(cpu_threads);
        builder.worker_threads(worker_threads);

        if self.enable_io {
            builder.enable_io();
        }

        if self.enable_time {
            builder.enable_time();
        }

        let counter = AtomicUsize::new(0);
        let thread_name = self.thread_name;
        // Deterministic, human-friendly thread names help debugging and metrics
        // attribution.
        builder.thread_name_fn(move || {
            let idx = counter.fetch_add(1, Ordering::SeqCst);
            format!("{thread_name}-{idx}")
        });

        builder.build().context(error::BuildSnafu)
    }
}

/// Build a single-threaded runtime with all drivers enabled.
///
/// # Errors
/// Returns an error if the runtime fails to build.
pub fn create_current_thread_runtime(thread_name: impl Into<String>) -> Result<Runtime> {
    let name = thread_name.into();
    let mut builder = TokioBuilder::new_current_thread();
    builder.enable_all();
    builder.thread_name(&name);
    builder.build().context(error::BuildSnafu)
}
