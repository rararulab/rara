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

//! Structured error reported for a failed LLM turn.
//!
//! A [`TurnError`] is emitted when a turn cannot produce a usable assistant
//! reply — either because the driver surfaced a [`crate::llm::StreamFailure`]
//! or because the iteration terminated with empty content and no tool calls
//! after all recovery attempts were exhausted.
//!
//! [`TurnError`] is published to the kernel event bus (event type
//! `turn.error`) so downstream consumers (telegram adapter, `/status`
//! endpoint, observability sinks) can surface the failure to the user rather
//! than leaving them staring at an empty "thinking" indicator.

use serde::{Deserialize, Serialize};

use crate::{llm::StreamFailure, session::SessionKey};

/// Event type label used when publishing [`TurnError`] to the kernel event
/// bus via [`crate::handle::KernelHandle::publish_event`].
pub const TURN_ERROR_EVENT_TYPE: &str = "turn.error";

/// Classification of the root cause of a failed turn.
///
/// This mirrors [`StreamFailure`] where possible, plus additional kinds that
/// are only detectable after the stream closes (e.g. an iteration ending
/// with no content and no tool calls despite a clean `Done` signal).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TurnFailureKind {
    /// The driver closed the stream without assistant content, even after
    /// attempting salvage from the reasoning buffer.
    EmptyContent {
        /// Size of the reasoning buffer at close. `0` when the failure was
        /// detected by the agent loop itself rather than by the driver.
        reasoning_len: usize,
    },
    /// The provider returned a non-retryable protocol error.
    ProtocolError {
        /// Provider-specific error code (e.g. MiniMax `"2013"`).
        code:    String,
        /// Provider-specific human-readable message.
        message: String,
    },
    /// The iteration terminated with no text and no tool calls after all
    /// recovery paths were exhausted (nudges, auto-fold + retry, etc.).
    EmptyTurn,
}

impl From<StreamFailure> for TurnFailureKind {
    fn from(value: StreamFailure) -> Self {
        match value {
            StreamFailure::EmptyContent { reasoning_len } => Self::EmptyContent { reasoning_len },
            StreamFailure::ProtocolError { code, message } => Self::ProtocolError { code, message },
        }
    }
}

/// Structured description of a failed turn, suitable for publishing on the
/// kernel event bus and surfacing to the user.
///
/// Construct via [`TurnError::builder`] and serialize into the event payload
/// with `serde_json::to_value`. The payload MUST include a non-empty
/// `message` field so the `PublishEvent` syscall does not drop it.
#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
pub struct TurnError {
    /// Session that owned the failed turn.
    pub session_key:  SessionKey,
    /// Model identifier used for the turn (e.g. `"minimax/m2"`).
    pub model:        String,
    /// Provider request ID when available — otherwise `None`.
    pub request_id:   Option<String>,
    /// Iteration index within the turn at which the failure surfaced.
    pub iteration:    usize,
    /// Structured failure classification.
    pub failure_kind: TurnFailureKind,
    /// Human-readable summary used as the notification `message` body.
    /// Required: the `PublishEvent` syscall drops payloads with an empty
    /// `message` field.
    pub message:      String,
}

impl TurnError {
    /// Short human-readable tag for logging and the `message` field.
    pub fn short_summary(failure_kind: &TurnFailureKind, model: &str) -> String {
        match failure_kind {
            TurnFailureKind::EmptyContent { reasoning_len } => {
                format!(
                    "model `{model}` produced no assistant content (reasoning_len={reasoning_len})"
                )
            }
            TurnFailureKind::ProtocolError { code, message } => {
                format!("provider `{model}` protocol error [{code}]: {message}")
            }
            TurnFailureKind::EmptyTurn => {
                format!("model `{model}` ended turn with no content and no tool calls")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_stream_failure_empty_content() {
        let kind: TurnFailureKind = StreamFailure::EmptyContent { reasoning_len: 42 }.into();
        assert_eq!(kind, TurnFailureKind::EmptyContent { reasoning_len: 42 });
    }

    #[test]
    fn from_stream_failure_protocol_error() {
        let kind: TurnFailureKind = StreamFailure::ProtocolError {
            code:    "2013".into(),
            message: "invalid message role: system".into(),
        }
        .into();
        assert_eq!(
            kind,
            TurnFailureKind::ProtocolError {
                code:    "2013".into(),
                message: "invalid message role: system".into(),
            }
        );
    }

    #[test]
    fn builder_round_trips_serde() {
        let err = TurnError::builder()
            .session_key(SessionKey::new())
            .model("minimax/m2".to_string())
            .iteration(3)
            .failure_kind(TurnFailureKind::EmptyTurn)
            .message("turn failed".to_string())
            .build();
        let value = serde_json::to_value(&err).expect("serialize");
        let back: TurnError = serde_json::from_value(value).expect("deserialize");
        assert_eq!(back.model, err.model);
        assert_eq!(back.iteration, err.iteration);
        assert_eq!(back.failure_kind, TurnFailureKind::EmptyTurn);
        assert_eq!(back.message, "turn failed");
    }
}
