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

//! Effects emitted by the sans-IO agent state machine.
//!
//! An [`Effect`] is a request from the pure [`crate::agent::machine`] for the
//! async [`crate::agent::runner`] to perform some side effect (LLM call, tool
//! invocation, tape append, …) and then feed the outcome back to the machine
//! as an [`crate::agent::machine::Event`].
//!
//! The split mirrors the *sans-IO* pattern used by `quinn-proto`: keeping the
//! state machine free of `.await` so it can be unit-tested synchronously
//! against any combination of events without spinning up real subsystems.

use crate::tool::ToolName;

/// Identifier for a single tool invocation issued by the LLM.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ToolCallId(pub String);

impl ToolCallId {
    /// Construct a [`ToolCallId`] from any string-like value.
    pub fn new(id: impl Into<String>) -> Self { Self(id.into()) }
}

/// One tool call requested by the LLM in the current iteration.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    /// Provider-assigned unique id for the call.
    pub id:        ToolCallId,
    /// Name of the tool to invoke.
    pub name:      ToolName,
    /// JSON-encoded argument string as emitted by the LLM.
    pub arguments: String,
}

/// Result of executing a single [`ToolCall`].
#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    /// Id of the originating tool call.
    pub id:          ToolCallId,
    /// Tool name (for tape persistence and metrics).
    pub name:        ToolName,
    /// Whether the tool ran to completion without error.
    pub success:     bool,
    /// Wall-clock duration of the call in milliseconds.
    pub duration_ms: u64,
    /// Optional human-readable error message when `success == false`.
    pub error:       Option<String>,
}

/// Side effects requested by the agent state machine.
///
/// Each variant corresponds to a real `.await` boundary in the legacy
/// `run_agent_loop`.  The runner is responsible for performing the effect and
/// translating the outcome into an [`crate::agent::machine::Event`] fed back
/// into [`crate::agent::machine::AgentMachine::step`].
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Issue a streaming LLM completion request for the current iteration.
    CallLlm {
        /// Zero-based iteration counter (informational).
        iteration:     usize,
        /// Whether tool calls are currently enabled.
        tools_enabled: bool,
    },
    /// Execute a batch of tool calls concurrently.
    RunTools {
        /// The calls to execute.
        calls: Vec<ToolCall>,
    },
    /// Append a structured entry to the tape.
    AppendTape {
        /// What is being persisted.
        kind: TapeAppendKind,
    },
    /// Emit a single user-facing stream event (progress, text delta, …).
    EmitStream {
        /// Type-erased stream payload (string for testability — the runner
        /// maps these onto real `StreamEvent`s).
        kind: String,
    },
    /// Terminate the loop and return a successful turn result.
    Finish {
        /// Final assistant text concatenated from the last iteration.
        text:       String,
        /// Number of iterations the machine actually ran.
        iterations: usize,
        /// Cumulative tool calls made across the turn.
        tool_calls: usize,
        /// Reason the machine reached a terminal state.
        reason:     FinishReason,
    },
    /// Inject a continuation wake system message into the conversation
    /// before the next LLM call. The runner constructs the message.
    InjectContinuationWake {
        /// Current continuation turn number (1-based).
        turn: usize,
        /// Maximum continuations allowed.
        max:  usize,
    },
    /// Terminate the loop with a failure.
    Fail {
        /// Free-form failure description.
        message: String,
    },
}

/// Categorisation of items the machine asks the runner to persist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TapeAppendKind {
    /// Final assistant message (turn terminator).
    AssistantFinal,
    /// Intermediate assistant message that preceded a tool wave.
    AssistantIntermediate,
    /// One or more tool call requests issued by the LLM.
    ToolCalls,
    /// One or more tool results returned to the LLM.
    ToolResults,
}

/// Why the machine terminated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason {
    /// The LLM produced a terminal response (no tool calls).
    Stopped,
    /// The configured maximum iterations was reached.
    MaxIterations,
    /// The user (or upstream limit) interrupted the turn.
    StoppedByLimit,
}
