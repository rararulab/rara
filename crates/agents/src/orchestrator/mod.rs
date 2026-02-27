pub mod context;
pub mod error;

mod core;
pub use core::AgentContextImpl;

/// Backward-compat alias.
pub type AgentOrchestrator = AgentContextImpl;
