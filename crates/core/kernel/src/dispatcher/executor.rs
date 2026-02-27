use async_trait::async_trait;

use super::types::AgentTask;
use crate::{agent_output::AgentOutput, error::KernelError};

/// Trait for executing dispatched agent tasks.
///
/// The kernel dispatcher delegates actual agent execution to an implementor
/// of this trait.  Concrete implementations (in the workers crate) handle
/// agent creation, session persistence, and post-execution callbacks.
#[async_trait]
pub trait TaskExecutor: Send + Sync + 'static {
    /// Execute a dispatched agent task and return the output.
    async fn execute(&self, task: &AgentTask) -> Result<AgentOutput, KernelError>;
}
