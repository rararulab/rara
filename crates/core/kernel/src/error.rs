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

use snafu::Snafu;
use uuid::Uuid;

#[derive(Debug, Snafu)]
pub enum KernelError {
    /// Agent not found in registry.
    #[snafu(display("agent not found: {id}"))]
    AgentNotFound { id: Uuid },

    /// Agent name already registered.
    #[snafu(display("agent already exists: {name}"))]
    AgentAlreadyExists { name: String },

    /// Agent runner error.
    #[snafu(display("agent runner error: {message}"))]
    Runner { message: String },

    /// Memory subsystem error.
    #[snafu(display("memory error: {message}"))]
    Memory { message: String },

    /// Kernel boot/initialization error.
    #[snafu(display("boot failed: {message}"))]
    Boot { message: String },
}

impl From<agent_core::err::Error> for KernelError {
    fn from(err: agent_core::err::Error) -> Self {
        Self::Runner {
            message: err.to_string(),
        }
    }
}

impl From<memory_core::MemoryError> for KernelError {
    fn from(err: memory_core::MemoryError) -> Self {
        Self::Memory {
            message: err.to_string(),
        }
    }
}

pub type Result<T> = std::result::Result<T, KernelError>;
