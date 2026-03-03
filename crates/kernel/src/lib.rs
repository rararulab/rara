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
//! | Memory      | [`Memory`]       | 3-layer memory (State/Knowledge/Learning) |
//! | Session     | [`SessionRepository`] | Conversation history persistence |
//! | Guard       | [`Guard`]        | Tool approval + output moderation |
//! | Notification Bus | [`NotificationBus`] | Inter-component notification broadcasting |

pub mod agent_turn;
pub mod audit;
pub mod channel;
pub(crate) mod delivery;
pub mod device;
pub mod error;
pub mod event;
pub mod event_loop;
pub mod guard;
pub mod handle;
pub mod io;
pub mod kernel;
pub mod kv;
pub mod llm;
pub mod memory;
pub mod metrics;
pub mod notification;
pub mod process;
pub mod queue;
pub mod security;
pub mod session;
pub(crate) mod syscall;
pub mod tool;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use audit::subsystem::AuditSubsystem;
pub use error::{KernelError, Result};
// New process model re-exports
pub use handle::{
    AgentHandle, kernel_handle::KernelHandle, process_handle::ProcessHandle,
    syscall_tool::SyscallTool,
};
pub use kernel::{Kernel, KernelConfig};
pub use process::{
    AgentId, AgentProcess, AgentRole, MetricsSnapshot, Priority, ProcessInfo, ProcessState,
    ProcessStats, ProcessTable, RuntimeMetrics, SandboxConfig, SessionId, Signal, SystemStats,
    agent_registry::AgentRegistry,
    manifest_loader::ManifestLoader,
    principal::{Principal, Role, UserId},
};
pub use security::{
    SecuritySubsystem,
    approval::{
        ApprovalDecision, ApprovalManager, ApprovalPolicy, ApprovalRequest, ApprovalResponse,
        RiskLevel,
    },
};

#[cfg(test)]
mod api_naming_tests {
    #[test]
    fn kernel_events_and_notifications_use_distinct_modules() {
        let _ = crate::event::KernelEvent::Shutdown;
        let _ = std::mem::size_of::<crate::notification::KernelNotification>();
    }

    #[test]
    fn queue_types_are_grouped_under_queue_module() {
        let _ = std::mem::size_of::<crate::queue::InMemoryEventQueue>();
        let _ = std::mem::size_of::<crate::queue::ShardedEventQueue>();
        let _ = std::mem::size_of::<crate::queue::ShardedEventQueueConfig>();
        let _ = std::mem::size_of::<crate::queue::EventQueueRef>();
    }

    #[test]
    fn event_processors_are_grouped_under_event_loop_module() {
        let _ = std::mem::size_of::<crate::event_loop::processor::EventProcessor>();
    }
}
