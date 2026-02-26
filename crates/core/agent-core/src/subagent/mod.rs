//! Sub-agent orchestration framework.
//!
//! This module implements a lightweight sub-agent system inspired by
//! [pi-mono](https://github.com/badlogic/pi-mono). The core idea: the main
//! agent can call a `"subagent"` tool to dispatch specialized child agents,
//! each with its own system prompt, model, and tool set.
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────┐
//! │  Main Agent (e.g. ChatService)                       │
//! │  - Has all tools including "subagent"                │
//! │  - Decides WHEN and HOW to use sub-agents            │
//! └──────────────────┬───────────────────────────────────┘
//!                    │ calls SubagentTool
//!                    ▼
//! ┌──────────────────────────────────────────────────────┐
//! │  SubagentTool                                        │
//! │  - Reads agent definitions from registry             │
//! │  - Dispatches to executor (single/chain/parallel)    │
//! │  - Returns structured SubagentResult as JSON         │
//! └──────────────────┬───────────────────────────────────┘
//!                    │ creates fresh AgentRunner per sub-agent
//!                    ▼
//! ┌──────────────────────────────────────────────────────┐
//! │  AgentRunner (per sub-agent)                         │
//! │  - Isolated context (own system prompt, history)     │
//! │  - Filtered tool set (never includes "subagent")     │
//! │  - Independent model and iteration limit             │
//! └──────────────────────────────────────────────────────┘
//! ```
//!
//! # Three Execution Modes
//!
//! 1. **Single** — Run one sub-agent with a task:
//!    `{"agent": "scout", "task": "Find all authentication code"}`
//!
//! 2. **Chain** — Run sub-agents sequentially. Each step can reference
//!    `{previous}` to receive the prior step's output. Stops on first error.
//!    `{"chain": [{"agent": "scout", "task": "Find relevant code"},`
//!    `{"agent": "planner", "task": "Create plan based on: {previous}"},`
//!    `{"agent": "worker", "task": "Implement: {previous}"}]}`
//!
//! 3. **Parallel** — Run sub-agents concurrently with a semaphore-based
//!    concurrency limit (default 4, max 8).
//!    `{"parallel": [{"agent": "scout", "task": "Find models"}, {"agent":`
//!    `"scout", "task": "Find providers"}]}`
//!
//! # Agent Definitions
//!
//! Agents are defined as markdown files with YAML frontmatter (same pattern
//! as skills). They live in the `agents/` directory:
//!
//! ```markdown
//! ---
//! name: scout
//! description: "Fast codebase recon"
//! model: "deepseek/deepseek-chat"   # optional, falls back to default
//! tools:                             # optional, empty = all parent tools
//!   - read_file
//!   - grep
//! max_iterations: 15                 # optional, default 15
//! ---
//!
//! You are a scout agent. Your system prompt goes here.
//! ```
//!
//! # Recursion Prevention
//!
//! Sub-agents never have access to the `"subagent"` tool itself. This is
//! enforced in two ways:
//! - The `parent_tools` snapshot is taken BEFORE `SubagentTool` is registered
//! - `build_subagent_tools()` explicitly filters out `"subagent"` from the tool
//!   set given to each sub-agent

pub mod builtin;
pub mod definition;
pub mod executor;
pub mod tool;

pub use builtin::all_bundled_agents;
pub use definition::{AgentDefinition, AgentDefinitionRegistry};
pub use executor::SubagentExecutor;
pub use tool::SubagentTool;
