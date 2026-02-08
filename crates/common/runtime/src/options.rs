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

use std::thread::available_parallelism;

use bon::Builder;
use smart_default::SmartDefault;

#[derive(Debug, Clone, SmartDefault, Builder)]
#[builder(finish_fn = build)]
pub struct GlobalRuntimeOptions {
    #[default(cpu_threads())]
    #[builder(default = cpu_threads())]
    pub file_io_threads: usize,

    #[default(cpu_threads())]
    #[builder(default = cpu_threads())]
    pub network_io_threads: usize,

    #[default(background_threads())]
    #[builder(default = background_threads())]
    pub background_threads: usize,
}

#[derive(Debug, Clone, SmartDefault, Builder)]
#[builder(finish_fn = build)]
pub struct RuntimeOptions {
    #[default(None)]
    pub worker_threads: Option<usize>,

    #[default("rsketch-rt".to_string())]
    #[builder(default = "rsketch-rt".to_string())]
    pub thread_name: String,

    #[default(true)]
    #[builder(default = true)]
    pub enable_io: bool,

    #[default(true)]
    #[builder(default = true)]
    pub enable_time: bool,
}

pub(crate) fn cpu_threads() -> usize {
    available_parallelism()
        .map(std::num::NonZero::get)
        .unwrap_or(1)
        .max(1)
}

pub(crate) fn background_threads() -> usize { cpu_threads().saturating_sub(1).max(1) }
