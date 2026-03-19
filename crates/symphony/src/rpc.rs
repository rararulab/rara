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

//! Ralph RPC protocol types for JSON-lines communication.

use serde::{Deserialize, Serialize};

/// Commands sent from Symphony to Ralph via stdin.
///
/// Each variant is serialized as a JSON object with a `"type"` tag
/// using snake_case naming.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcCommand {
    /// Inject guidance that Ralph will consider on its next iteration.
    Guidance {
        /// Optional correlation ID for request-response matching.
        #[serde(skip_serializing_if = "Option::is_none")]
        id:      Option<String>,
        /// The guidance message content.
        message: String,
    },
    /// Immediately inject a steering message into Ralph's current iteration.
    Steer {
        /// Optional correlation ID for request-response matching.
        #[serde(skip_serializing_if = "Option::is_none")]
        id:      Option<String>,
        /// The steering message content.
        message: String,
    },
    /// Terminate Ralph's agentic loop.
    Abort {
        /// Optional correlation ID for request-response matching.
        #[serde(skip_serializing_if = "Option::is_none")]
        id:     Option<String>,
        /// Optional reason for the abort.
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// Request a snapshot of Ralph's current state.
    GetState {
        /// Optional correlation ID for request-response matching.
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
}

/// Events emitted from Ralph to Symphony via stdout.
///
/// Each variant is deserialized from a JSON object with a `"type"` tag
/// using snake_case naming.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcEvent {
    /// Ralph's agentic loop has started.
    LoopStarted {
        /// The initial prompt that started the loop.
        prompt:  String,
        /// The LLM backend in use.
        backend: String,
    },
    /// A new iteration within the loop is beginning.
    IterationStart {
        /// Zero-based iteration index.
        iteration: u32,
        /// The current hat (role) Ralph is wearing.
        hat:       String,
        /// The LLM backend in use.
        backend:   String,
    },
    /// An iteration has completed.
    IterationEnd {
        /// Zero-based iteration index.
        iteration:               u32,
        /// Wall-clock duration of this iteration in milliseconds.
        duration_ms:             u64,
        /// Estimated cost in USD.
        cost_usd:                f64,
        /// Number of input tokens consumed.
        input_tokens:            u64,
        /// Number of output tokens produced.
        output_tokens:           u64,
        /// Whether the loop-complete condition was triggered.
        loop_complete_triggered: bool,
    },
    /// A chunk of streaming text from the LLM.
    TextDelta {
        /// The iteration this delta belongs to.
        iteration: u32,
        /// The text fragment.
        delta:     String,
    },
    /// A tool call is starting.
    ToolCallStart {
        /// The iteration this tool call belongs to.
        iteration:    u32,
        /// Name of the tool being invoked.
        tool_name:    String,
        /// Unique identifier for this tool call.
        tool_call_id: String,
        /// The input arguments passed to the tool.
        input:        serde_json::Value,
    },
    /// A tool call has completed.
    ToolCallEnd {
        /// The iteration this tool call belongs to.
        iteration:    u32,
        /// Unique identifier matching the corresponding `ToolCallStart`.
        tool_call_id: String,
        /// The tool's output.
        output:       String,
        /// Whether the tool reported an error.
        is_error:     bool,
        /// Wall-clock duration of the tool call in milliseconds.
        duration_ms:  u64,
    },
    /// An error occurred during an iteration.
    #[serde(rename = "error")]
    Error {
        /// The iteration where the error occurred.
        iteration:   u32,
        /// Machine-readable error code.
        code:        String,
        /// Human-readable error description.
        message:     String,
        /// Whether Ralph can recover from this error.
        recoverable: bool,
    },
    /// Ralph switched hats (roles) mid-loop.
    HatChanged {
        /// The iteration where the change occurred.
        iteration:      u32,
        /// Previous hat identifier.
        from_hat:       String,
        /// New hat identifier.
        to_hat:         String,
        /// Display name for the new hat.
        to_hat_display: String,
        /// Reason for the hat change.
        reason:         String,
    },
    /// A task's status changed during the loop.
    TaskStatusChanged {
        /// Identifier of the task whose status changed.
        task_id:     String,
        /// Previous status.
        from_status: String,
        /// New status.
        to_status:   String,
        /// Human-readable task title.
        title:       String,
    },
    /// Acknowledgement that guidance was received by Ralph.
    GuidanceAck {
        /// The guidance message that was acknowledged.
        message:    String,
        /// Which iteration or phase the guidance applies to.
        applies_to: String,
    },
    /// Ralph's agentic loop has terminated.
    LoopTerminated {
        /// Reason for termination (e.g. "completed", "aborted",
        /// "max_iterations").
        reason:           String,
        /// Total number of iterations executed.
        total_iterations: u32,
        /// Total wall-clock duration in milliseconds.
        duration_ms:      u64,
        /// Total estimated cost in USD.
        total_cost_usd:   f64,
    },
    /// Response to a command (e.g. `GetState`).
    Response {
        /// The command type this is responding to.
        command: String,
        /// Correlation ID from the original command.
        #[serde(default)]
        id:      Option<String>,
        /// Whether the command succeeded.
        success: bool,
        /// Optional response payload.
        #[serde(default)]
        data:    Option<serde_json::Value>,
        /// Optional error message if `success` is false.
        #[serde(default)]
        error:   Option<String>,
    },
}

impl RpcEvent {
    /// Returns `true` if this event signals the end of Ralph's loop.
    pub fn is_terminal(&self) -> bool { matches!(self, RpcEvent::LoopTerminated { .. }) }

    /// Extracts the termination reason if this is a `LoopTerminated` event.
    pub fn termination_reason(&self) -> Option<&str> {
        match self {
            RpcEvent::LoopTerminated { reason, .. } => Some(reason),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_guidance_command() {
        let cmd = RpcCommand::Guidance {
            id:      Some("req-1".into()),
            message: "focus on tests".into(),
        };
        let json = serde_json::to_string(&cmd).expect("serialize");
        assert!(json.contains(r#""type":"guidance""#));
        assert!(json.contains("focus on tests"));
    }

    #[test]
    fn serialize_abort_command() {
        let cmd = RpcCommand::Abort {
            id:     None,
            reason: Some("user cancelled".into()),
        };
        let json = serde_json::to_string(&cmd).expect("serialize");
        assert!(json.contains(r#""type":"abort""#));
        assert!(json.contains("user cancelled"));
        // id should be omitted entirely
        assert!(!json.contains(r#""id""#));
    }

    #[test]
    fn deserialize_loop_terminated_event() {
        let json = r#"{
            "type": "loop_terminated",
            "reason": "completed",
            "total_iterations": 5,
            "duration_ms": 12000,
            "total_cost_usd": 0.42
        }"#;
        let event: RpcEvent = serde_json::from_str(json).expect("deserialize");
        assert!(event.is_terminal());
        assert_eq!(event.termination_reason(), Some("completed"));
    }

    #[test]
    fn deserialize_iteration_end_event() {
        let json = r#"{
            "type": "iteration_end",
            "iteration": 2,
            "duration_ms": 3500,
            "cost_usd": 0.08,
            "input_tokens": 1200,
            "output_tokens": 450,
            "loop_complete_triggered": false
        }"#;
        let event: RpcEvent = serde_json::from_str(json).expect("deserialize");
        assert!(!event.is_terminal());
        assert_eq!(event.termination_reason(), None);
    }

    #[test]
    fn deserialize_unknown_fields_are_ignored() {
        let json = r#"{
            "type": "loop_terminated",
            "reason": "max_iterations",
            "total_iterations": 10,
            "duration_ms": 60000,
            "total_cost_usd": 1.5,
            "extra_field": "should be ignored",
            "another_unknown": 42
        }"#;
        let event: RpcEvent = serde_json::from_str(json).expect("deserialize");
        assert!(event.is_terminal());
        assert_eq!(event.termination_reason(), Some("max_iterations"));
    }
}
