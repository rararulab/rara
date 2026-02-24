// Copyright 2025 Crrow
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

//! Error types for task agents.

use snafu::Snafu;

/// Errors that can occur during task agent operations.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum TaskAgentError {
    /// No AI provider is configured.
    #[snafu(display("AI provider not configured"))]
    NotConfigured,

    /// An AI provider request failed.
    #[snafu(display("AI request failed: {message}"))]
    RequestFailed { message: String },

    /// The AI response was empty or contained no usable content.
    #[snafu(display("AI returned empty response"))]
    EmptyResponse,

    /// A tool call failed during tool-calling mode.
    #[snafu(display("tool call failed: {message}"))]
    ToolCallFailed { message: String },

    /// The tool-calling loop exceeded its maximum iteration count.
    #[snafu(display("tool-calling loop exceeded {max} iterations"))]
    MaxIterationsExceeded { max: usize },
}
