//! Built-in agents -- Rust-defined agents with lifecycle hooks.
//!
//! Each built-in agent encapsulates its own prompt strategy, tool selection,
//! and pre/post processing. The lifecycle hooks (`prepare` / `post_process`)
//! are intentionally minimal now; #190 will fill them with Goal/Task/Journal
//! capabilities.

pub mod chat;
pub mod proactive;
pub mod scheduled;
pub mod tasks;

/// Output from a built-in agent execution.
pub struct AgentOutput {
    /// The assistant's response text.
    pub response_text: String,
    /// Number of LLM iterations used.
    pub iterations: usize,
    /// Number of tool calls made.
    pub tool_calls_made: usize,
    /// `true` when the agent loop was stopped early because it hit the
    /// max-iterations ceiling. The response contains all work completed
    /// so far, but the task may be incomplete.
    pub truncated: bool,
}

impl AgentOutput {
    /// Build from an [`AgentRunResponse`], extracting the response text.
    pub fn from_run_response(response: &agent_core::runner::AgentRunResponse) -> Self {
        Self {
            response_text: response.response_text(),
            iterations: response.iterations,
            tool_calls_made: response.tool_calls_made,
            truncated: response.truncated,
        }
    }
}
