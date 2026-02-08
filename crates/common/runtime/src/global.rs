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

use std::{future::Future, sync::Arc};

use once_cell::sync::OnceCell;
use tokio::{runtime::Runtime, task::JoinHandle};

use crate::options::{GlobalRuntimeOptions, RuntimeOptions};

#[derive(Debug)]
struct GlobalRuntimes {
    file_io:    Arc<Runtime>,
    network_io: Arc<Runtime>,
    background: Arc<Runtime>,
}

static GLOBAL_RUNTIMES: OnceCell<GlobalRuntimes> = OnceCell::new();

fn build_global_runtimes(options: &GlobalRuntimeOptions) -> GlobalRuntimes {
    let file_io = Arc::new(
        RuntimeOptions::builder()
            .thread_name("rt-file-io".to_string())
            .worker_threads(options.file_io_threads)
            .enable_io(true)
            .enable_time(true)
            .build()
            .create()
            .expect("Failed to create file-io runtime"),
    );
    let network_io = Arc::new(
        RuntimeOptions::builder()
            .thread_name("rt-net-io".to_string())
            .worker_threads(options.network_io_threads)
            .enable_io(true)
            .enable_time(true)
            .build()
            .create()
            .expect("Failed to create network-io runtime"),
    );
    let background = Arc::new(
        RuntimeOptions::builder()
            .thread_name("rt-bg".to_string())
            .worker_threads(options.background_threads)
            .enable_io(true)
            .enable_time(true)
            .build()
            .create()
            .expect("Failed to create background runtime"),
    );

    GlobalRuntimes {
        file_io,
        network_io,
        background,
    }
}

fn global_runtimes() -> &'static GlobalRuntimes {
    GLOBAL_RUNTIMES.get_or_init(|| build_global_runtimes(&GlobalRuntimeOptions::default()))
}

/// Initialize global runtimes with custom options.
///
/// # Panics
/// Panics if called more than once.
pub fn init_global_runtimes(options: &GlobalRuntimeOptions) {
    GLOBAL_RUNTIMES
        .set(build_global_runtimes(options))
        .expect("Global runtimes already initialized");
}

#[must_use]
pub fn file_io_runtime() -> Arc<Runtime> { Arc::clone(&global_runtimes().file_io) }

#[must_use]
pub fn network_io_runtime() -> Arc<Runtime> { Arc::clone(&global_runtimes().network_io) }

#[must_use]
pub fn background_runtime() -> Arc<Runtime> { Arc::clone(&global_runtimes().background) }

pub fn spawn_file_io<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    file_io_runtime().handle().spawn(future)
}

pub fn spawn_blocking_file_io<F, R>(job: F) -> JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    file_io_runtime().handle().spawn_blocking(job)
}

pub fn block_on_file_io<F>(future: F) -> F::Output
where
    F: Future,
{
    file_io_runtime().block_on(future)
}

pub fn spawn_network_io<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    network_io_runtime().handle().spawn(future)
}

pub fn spawn_blocking_network_io<F, R>(job: F) -> JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    network_io_runtime().handle().spawn_blocking(job)
}

pub fn block_on_network_io<F>(future: F) -> F::Output
where
    F: Future,
{
    network_io_runtime().block_on(future)
}

pub fn spawn_background<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    background_runtime().handle().spawn(future)
}

pub fn spawn_blocking_background<F, R>(job: F) -> JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    background_runtime().handle().spawn_blocking(job)
}

pub fn block_on_background<F>(future: F) -> F::Output
where
    F: Future,
{
    background_runtime().block_on(future)
}
