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

//! Streaming event types for LLM completion.

use super::types::{StopReason, Usage};

/// Typed failure reasons reported by a driver at stream close.
///
/// Emitted as part of [`StreamDelta::Failure`] when the driver completes a
/// stream but detects a condition that would otherwise silently produce an
/// empty or malformed assistant turn. Consumers may use these signals to
/// surface errors to the user instead of writing empty tape records.
#[derive(Debug, Clone)]
pub enum StreamFailure {
    /// The provider closed the stream with no assistant content despite
    /// emitting `reasoning_content` — typically MiniMax-M2 finishing the
    /// `<think>` block and then hitting EOS before any real answer. The
    /// driver already attempted salvage (extracting text after the last
    /// `</think>` tag) and either found nothing or only whitespace.
    EmptyContent {
        /// Number of characters in the reasoning buffer at stream close —
        /// useful for diagnostics and for consumers deciding whether to
        /// retry vs. surface the failure.
        reasoning_len: usize,
    },
    /// The provider returned a non-retryable protocol error (e.g. MiniMax
    /// `system (2013)` HTTP 400). The driver propagates the provider's
    /// error code and human-readable message.
    ProtocolError {
        /// Provider-specific error code (e.g. `"2013"`).
        code:    String,
        /// Provider-specific human-readable message.
        message: String,
    },
}

/// Events emitted during streaming LLM completion.
///
/// The LLM driver sends these through an `mpsc::Sender<StreamDelta>` as
/// SSE chunks arrive from the provider.
#[derive(Debug, Clone)]
pub enum StreamDelta {
    /// Incremental text content.
    TextDelta { text: String },
    /// Incremental reasoning/thinking content (e.g. DeepSeek-R1 thinking
    /// tokens).
    ReasoningDelta { text: String },
    /// A tool call has started — id and name are known.
    ToolCallStart {
        index: u32,
        id:    String,
        name:  String,
    },
    /// Incremental JSON fragment for an in-progress tool call's arguments.
    ToolCallArgumentsDelta { index: u32, arguments: String },
    /// A typed failure signal emitted by the driver before `Done`.
    ///
    /// Consumers that do not understand a specific failure kind should log
    /// and fall through — the following `Done` event will still close the
    /// stream.
    Failure(StreamFailure),
    /// The stream is complete.
    Done {
        stop_reason: StopReason,
        usage:       Option<Usage>,
    },
}
