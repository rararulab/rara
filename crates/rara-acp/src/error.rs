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

use snafu::Snafu;

/// Errors produced by ACP (Agent Communication Protocol) operations.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum AcpError {
    /// Failed to spawn the child agent process.
    #[snafu(display("Failed to spawn agent process: {source}"))]
    SpawnProcess { source: std::io::Error },

    /// The ACP initialize handshake did not complete successfully.
    #[snafu(display("ACP initialize handshake failed: {source}"))]
    Initialize {
        source: agent_client_protocol::Error,
    },

    /// Session creation failed on the remote agent.
    #[snafu(display("ACP session creation failed: {source}"))]
    NewSession {
        source: agent_client_protocol::Error,
    },

    /// The ACP handshake failed due to a local setup problem (e.g. missing
    /// stdio pipe) rather than a protocol-level error from the remote agent.
    #[snafu(display("ACP handshake failed: {message}"))]
    Handshake { message: String },

    /// A low-level protocol or transport error occurred.
    #[snafu(display("ACP protocol error: {message}"))]
    Protocol { message: String },

    /// The requested session does not exist or has already been closed.
    #[snafu(display("Session not found: {session_id}"))]
    SessionNotFound { session_id: String },

    /// The remote agent process exited unexpectedly.
    #[snafu(display("Agent process exited unexpectedly: {message}"))]
    AgentExited { message: String },

    /// The agent rejected or failed to process the prompt.
    #[snafu(display("Prompt failed: {source}"))]
    PromptFailed {
        source: agent_client_protocol::Error,
    },

    /// The remote agent advertises an ACP version we cannot speak.
    #[snafu(display("Unsupported ACP version: {version}"))]
    UnsupportedVersion { version: String },

    /// The requested tool call does not exist in the thread.
    #[snafu(display("Tool call not found: {tool_call_id}"))]
    ToolCallNotFound { tool_call_id: String },

    /// Failed to load or parse the registry from disk.
    #[snafu(display("Failed to load ACP registry: {source}"))]
    RegistryLoad { source: std::io::Error },

    /// Failed to parse the registry JSON.
    #[snafu(display("Failed to parse ACP registry: {source}"))]
    RegistryParse { source: serde_json::Error },

    /// Failed to save the registry to disk.
    #[snafu(display("Failed to save ACP registry: {source}"))]
    RegistrySave { source: std::io::Error },

    /// Failed to serialize the registry to JSON.
    #[snafu(display("Failed to serialize ACP registry: {source}"))]
    RegistrySerialize { source: serde_json::Error },

    /// An operation on a builtin agent was rejected.
    #[snafu(display("Builtin agent protection: {message}"))]
    BuiltinProtection { message: String },

    /// The registry path was not configured.
    #[snafu(display("ACP registry path not configured"))]
    RegistryPathNotSet,
}
