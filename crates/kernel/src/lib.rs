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
//! | LLM         | [`LlmDriver`]    | Chat completion requests         |
//! | Tool        | [`ToolRegistry`] | Tool registration + dispatch     |
//! | Memory      | (tape-based)     | Agent memory via tape subsystem   |
//! | Session     | [`SessionIndex`] | Session metadata persistence      |
//! | Guard       | [`Guard`]        | Tool approval + output moderation |
//! | Notification Bus | [`NotificationBus`] | Inter-component notification broadcasting |

pub mod agent_loop;
pub mod channel;
// TODO: deprecate me
pub mod compaction;
pub(crate) mod delivery;
pub mod error;
pub mod event;
pub mod event_loop;
pub mod handle;
pub mod handle;
pub mod io;
pub mod kernel;
pub mod kv;
pub mod llm;
pub mod metrics;
pub mod notification;
pub mod process;
pub mod queue;
pub mod security;
pub mod session;
pub(crate) mod syscall;
pub mod tool;

pub use error::{KernelError, Result};
// Session-centric runtime re-exports (new names + backwards-compatible aliases)
pub use handle::{
    AgentHandle, kernel_handle::KernelHandle, process_handle::ProcessHandle,
    syscall_tool::SyscallTool,
};
pub use kernel::{Kernel, KernelConfig};
pub use process::{
    // New canonical names

    // Backwards-compatible aliases
    AgentRole,
    MetricsSnapshot,
    Priority,
    ProcessTable,
    RuntimeMetrics,
    SandboxConfig,
    SessionInfo,

    SessionRuntime,
    SessionState,
    SessionStats,
    SessionTable,
    Signal,
    SystemStats,
    agent_registry::AgentRegistry,
    manifest_loader::ManifestLoader,
    principal::{Principal, Role, UserId},
};
pub use security::{
    ApprovalDecision, ApprovalManager, ApprovalPolicy, ApprovalRequest, ApprovalResponse,
    RiskLevel, SecuritySubsystem,
};
