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
    /// The stream is complete.
    Done {
        stop_reason: StopReason,
        usage:       Option<Usage>,
    },
}
