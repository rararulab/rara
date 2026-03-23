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

/// ACP (Agent Communication Protocol) client for Rara.
///
/// This crate provides a Rust client that speaks the ACP protocol to
/// communicate with external agent processes (spawn, handshake, prompt,
/// session management).
pub mod connection;
pub mod delegate;
pub mod error;
pub mod events;
pub mod registry;
pub mod thread;

// Re-export commonly used types from agent_client_protocol so downstream
// crates don't need a direct dependency on the protocol crate.
pub use agent_client_protocol::{RequestPermissionOutcome, SelectedPermissionOutcome};
pub use connection::{AcpConnection, AgentCommand};
pub use error::AcpError;
pub use events::{
    AcpEvent, FileOperation, PermissionBridge, PermissionOptionInfo, PermissionOptionKind,
    StopReason, ToolCallStatus,
};
pub use registry::{AcpAgentConfig, AcpRegistry, AcpRegistryRef, FSAcpRegistry};
pub use thread::{
    AcpThread, AcpThreadEntry, AcpThreadStatus, AcpToolCall, PermissionPolicy,
    PermissionRequestInfo,
};
