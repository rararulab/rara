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

pub use connection::AcpConnection;
pub use error::AcpError;
pub use events::{AcpEvent, FileOperation, StopReason, ToolCallStatus};
pub use registry::{AgentCommand, AgentKind, AgentRegistry};
