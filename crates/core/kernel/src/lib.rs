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

//! # rara-kernel
//!
//! Unified agent orchestrator — the single core crate for agent lifecycle,
//! 7-component architecture, and LLM ↔ Tool execution loop.
//!
//! ## 6 Components
//!
//! | Component   | Trait            | Purpose                          |
//! |-------------|------------------|----------------------------------|
//! | LLM         | [`LlmApi`]       | Chat completion requests         |
//! | Tool        | [`ToolRegistry`] | Tool registration + dispatch     |
//! | Memory      | [`Memory`]       | 3-layer memory (State/Knowledge/Learning) |
//! | Session     | [`SessionRepository`] | Conversation history persistence |
//! | Guard       | [`Guard`]        | Tool approval + output moderation |
//! | Event Bus   | [`EventBus`]     | Inter-component event broadcasting |

pub mod agent_turn;
pub mod approval;
pub mod audit;
pub mod channel;
pub mod defaults;
pub mod device;
pub mod device_registry;
pub mod error;
pub mod event;
pub mod event_loop;
pub(crate) mod event_processor;
pub mod event_queue;
pub mod guard;
pub mod handle;
pub mod io;
pub mod kernel;
pub mod kv;
pub mod llm;
pub mod memory;
pub mod metrics;
pub mod model;
pub mod process;
pub mod provider;
pub mod runner;
pub mod session;
pub(crate) mod shard_queue;
pub mod sharded_event_queue;
pub mod tool;
pub mod unified_event;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use approval::{
    ApprovalDecision, ApprovalManager, ApprovalPolicy, ApprovalRequest, ApprovalResponse, RiskLevel,
};
pub use error::{KernelError, Result};
// New process model re-exports
pub use handle::{AgentHandle, process_handle::ProcessHandle, syscall_tool::SyscallTool};
pub use kernel::{Kernel, KernelConfig};
pub use process::{
    AgentId, AgentProcess, AgentRole, MetricsSnapshot, Priority, ProcessInfo, ProcessState,
    ProcessStats, ProcessTable, RuntimeMetrics, SandboxConfig, SessionId, Signal, SystemStats,
    agent_registry::AgentRegistry,
    manifest_loader::ManifestLoader,
    principal::{Principal, Role, UserId},
};
