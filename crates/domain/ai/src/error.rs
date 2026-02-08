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

/// Errors that can occur during AI provider operations.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum AiError {
    /// The provider returned a rate-limit (HTTP 429) response.
    #[snafu(display("Rate limited by {provider}: retry after {retry_after_secs}s"))]
    RateLimited {
        provider:         String,
        retry_after_secs: u64,
    },

    /// Authentication with the provider failed (invalid or expired API
    /// key).
    #[snafu(display("Authentication failed for provider {provider}: {message}"))]
    AuthFailed { provider: String, message: String },

    /// The requested model is not available or does not exist.
    #[snafu(display("Model unavailable: {model} on {provider}"))]
    ModelUnavailable { provider: String, model: String },

    /// The provider returned a response that could not be parsed.
    #[snafu(display("Invalid response from {provider}: {message}"))]
    InvalidResponse { provider: String, message: String },

    /// The request exceeded the model's context window.
    #[snafu(display("Context length exceeded for model {model}: {message}"))]
    ContextLengthExceeded { model: String, message: String },

    /// A prompt template variable was missing during rendering.
    #[snafu(display("Missing template variable: {variable}"))]
    MissingTemplateVariable { variable: String },

    /// The requested prompt template was not found.
    #[snafu(display("Prompt template not found: {name}"))]
    TemplateNotFound { name: String },

    /// No provider is configured for the requested task.
    #[snafu(display("No provider configured for task kind: {kind}"))]
    NoProviderConfigured { kind: String },

    /// A network or transport error occurred while calling the
    /// provider.
    #[snafu(display("Provider request failed: {message}"))]
    RequestFailed { message: String },

    /// Catch-all for unexpected internal errors.
    #[snafu(display("Internal AI error: {message}"))]
    Internal { message: String },
}
