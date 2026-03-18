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
pub use connection::AcpConnection;
pub use error::AcpError;
pub use events::{
    AcpEvent, FileOperation, PermissionBridge, PermissionOptionInfo, StopReason, ToolCallStatus,
};
pub use registry::{AgentCommand, AgentKind, AgentRegistry};
pub use thread::{AcpThread, AcpThreadEntry, AcpThreadStatus, AcpToolCall, PermissionRequestInfo};
