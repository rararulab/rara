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

//! Error types for the AI domain crate.

use snafu::Snafu;

/// Errors that can occur during AI operations.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum AiError {
    /// The rate limiter rejected the request.
    #[snafu(display("Rate limited by {provider}: retry after {retry_after_secs}s"))]
    RateLimited {
        provider:         String,
        retry_after_secs: u64,
    },

    /// A prompt template variable was missing during rendering.
    #[snafu(display("Missing template variable: {variable}"))]
    MissingTemplateVariable { variable: String },

    /// The requested prompt template was not found.
    #[snafu(display("Prompt template not found: {name}"))]
    TemplateNotFound { name: String },

    /// No AI provider is configured.
    #[snafu(display("AI provider not configured"))]
    NotConfigured,

    /// An AI provider request failed.
    #[snafu(display("AI request failed: {message}"))]
    RequestFailed { message: String },
}
