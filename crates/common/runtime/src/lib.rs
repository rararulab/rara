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

mod error;
mod factory;
mod global;
mod options;

pub use error::{Error, Result};
pub use factory::create_current_thread_runtime;
pub use global::{
    background_runtime, block_on_background, block_on_file_io, block_on_network_io,
    file_io_runtime, init_global_runtimes, network_io_runtime, spawn_background,
    spawn_blocking_background, spawn_blocking_file_io, spawn_blocking_network_io, spawn_file_io,
    spawn_network_io,
};
pub use options::{GlobalRuntimeOptions, RuntimeOptions};
pub use tokio::{runtime::Runtime, task::JoinHandle};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_multi_thread_runtime_with_names() {
        let runtime = RuntimeOptions::builder()
            .thread_name("test-rt".to_string())
            .worker_threads(2)
            .build()
            .create()
            .unwrap();
        let handle = runtime.spawn(async move { std::thread::current().name().map(str::to_owned) });
        let handle_name = runtime.block_on(handle).unwrap().unwrap();
        assert!(handle_name.starts_with("test-rt-"));
    }

    #[test]
    fn builds_current_thread_runtime() {
        let runtime = create_current_thread_runtime("single-thread").unwrap();
        let value = runtime.block_on(async { 42 });
        assert_eq!(value, 42);
    }

    #[test]
    fn builds_predefined_runtimes() {
        let file_rt = RuntimeOptions::builder()
            .worker_threads(2)
            .thread_name("rt-file-io".to_string())
            .enable_io(true)
            .enable_time(true)
            .build()
            .create()
            .unwrap();
        let net_rt = RuntimeOptions::builder()
            .thread_name("rt-net-io".to_string())
            .enable_io(true)
            .enable_time(true)
            .build()
            .create()
            .unwrap();
        let bg_rt = RuntimeOptions::builder()
            .worker_threads(1)
            .thread_name("rt-bg".to_string())
            .enable_io(true)
            .enable_time(true)
            .build()
            .create()
            .unwrap();

        let file_name = file_rt
            .block_on(file_rt.spawn(async { std::thread::current().name().map(str::to_owned) }))
            .unwrap()
            .unwrap_or_default();
        let net_name = net_rt
            .block_on(net_rt.spawn(async { std::thread::current().name().map(str::to_owned) }))
            .unwrap()
            .unwrap_or_default();
        let bg_name = bg_rt
            .block_on(bg_rt.spawn(async { std::thread::current().name().map(str::to_owned) }))
            .unwrap()
            .unwrap_or_default();

        assert!(file_name.starts_with("rt-file-io-"));
        assert!(net_name.starts_with("rt-net-io-"));
        assert!(bg_name.starts_with("rt-bg-"));
    }

    #[test]
    fn global_runtimes_can_spawn() {
        init_global_runtimes(&GlobalRuntimeOptions {
            file_io_threads:    1,
            network_io_threads: 1,
            background_threads: 1,
        });

        let handle = spawn_file_io(async { 5 });
        let value = block_on_file_io(handle).unwrap();
        assert_eq!(value, 5);

        let net = block_on_network_io(async { 7 });
        assert_eq!(net, 7);

        let bg = block_on_background(async { 11 });
        assert_eq!(bg, 11);
    }
}
