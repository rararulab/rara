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
