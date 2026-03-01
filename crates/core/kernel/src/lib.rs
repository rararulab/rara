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

//! # rara-kernel
//!
//! Unified agent orchestrator — the single core crate for agent lifecycle,
//! 7-component architecture, and LLM ↔ Tool execution loop.
//!
//! ## 7 Components
//!
//! | Component   | Trait            | Purpose                          |
//! |-------------|------------------|----------------------------------|
//! | LLM         | [`LlmApi`]       | Chat completion requests         |
//! | Tool        | [`ToolRegistry`] | Tool registration + dispatch     |
//! | Memory      | [`Memory`]       | 3-layer memory (State/Knowledge/Learning) |
//! | Session     | [`SessionRepository`] | Conversation history persistence |
//! | Prompt      | [`PromptRepo`]   | System prompt templates          |
//! | Guard       | [`Guard`]        | Tool approval + output moderation |
//! | Event Bus   | [`EventBus`]     | Inter-component event broadcasting |

pub mod agent_output;
pub mod audit;
pub mod channel;
pub mod defaults;
pub mod error;
pub mod event;
pub mod guard;
pub mod handle;
pub mod io;
pub mod kernel;
pub mod llm;
pub mod memory;
pub mod model;
pub mod model_repo;
pub mod process;
pub mod process_loop;
pub mod prompt;
pub mod provider;
pub mod runner;
pub mod session;
pub mod tick;
pub mod tool;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use error::{KernelError, Result};
// New process model re-exports
pub use handle::{AgentHandle, EventOps, GuardOps, KernelHandle, MemoryOps, ProcessOps};
pub use handle::{scoped::ScopedKernelHandle, spawn_tool::SpawnTool};
pub use kernel::{Kernel, KernelConfig};
pub use process::{
    AgentId, AgentProcess, ProcessInfo, ProcessMessage, ProcessState, ProcessTable, SessionId,
    Signal,
    manifest_loader::ManifestLoader,
    principal::{Principal, Role, UserId},
};
