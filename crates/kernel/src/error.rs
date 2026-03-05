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

use base::shared_string::SharedString;
use snafu::Snafu;
use uuid::Uuid;

use crate::session::SessionKey;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum KernelError {
    /// Session runtime not found.
    #[snafu(display("session runtime not found: {key}"))]
    SessionRuntimeNotFound { key: SessionKey },

    /// Agent name already registered.
    #[snafu(display("agent already exists: {name}"))]
    AgentAlreadyExists { name: String },

    /// LLM provider error.
    #[snafu(display("llm error: {message}"))]
    Llm { message: String },

    /// Memory subsystem error.
    #[snafu(display("memory error: {message}"))]
    Memory { message: String },

    /// Session store error.
    #[snafu(display("session error: {message}"))]
    Session { message: String },

    /// Guard error.
    #[snafu(display("guard error: {message}"))]
    Guard { message: String },

    /// Event bus error.
    #[snafu(display("event error: {message}"))]
    Event { message: String },

    /// Tool registry error.
    #[snafu(display("tool error: {message}"))]
    Tool { message: String },

    /// Kernel boot/initialization error.
    #[snafu(display("boot failed: {message}"))]
    Boot { message: String },

    // -- Provider-related errors (moved from agent-core) -----------------------
    #[snafu(display("LLM provider error: {message}"))]
    Provider { message: SharedString },

    #[snafu(display("LLM provider not configured"))]
    ProviderNotConfigured {
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("context window exceeded"))]
    ContextWindow,

    #[snafu(display("retryable server error"))]
    RetryableServer,

    #[snafu(display("non-retryable error"))]
    NonRetryable,

    #[snafu(display("{}", source))]
    Io {
        source:   std::io::Error,
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("context error: {message}"))]
    Context { message: SharedString },

    #[snafu(display("{message}"))]
    Other { message: SharedString },

    /// Agent execution failed.
    #[snafu(display("agent execution failed: {message}"))]
    AgentExecution { message: String },

    /// Process not found in process table.
    #[snafu(display("process not found: {id}"))]
    ProcessNotFound { id: String },

    /// Process is in a terminal state (Completed/Failed/Cancelled).
    #[snafu(display("process terminal: {id} ({state})"))]
    ProcessTerminal { id: String, state: String },

    /// Permission denied for the requested operation.
    #[snafu(display("permission denied: {reason}"))]
    PermissionDenied { reason: String },

    /// Spawn limit reached (global or per-agent).
    #[snafu(display("spawn limit reached: {message}"))]
    SpawnLimitReached { message: String },

    /// Tool not allowed for the agent.
    #[snafu(display("tool not allowed: {tool_name}"))]
    ToolNotAllowed { tool_name: String },

    /// Manifest not found by name.
    #[snafu(display("manifest not found: {name}"))]
    ManifestNotFound { name: String },

    /// Agent process was cancelled or failed before producing a result.
    #[snafu(display("spawn failed: {message}"))]
    SpawnFailed { message: String },

    /// User not found in user store.
    #[snafu(display("user not found: {name}"))]
    UserNotFound { name: String },

    /// User account is disabled.
    #[snafu(display("user disabled: {name}"))]
    UserDisabled { name: String },

    /// Memory scope access denied.
    #[snafu(display("memory scope denied: {reason}"))]
    MemoryScopeDenied { reason: String },

    /// Memory quota exceeded for a session.
    #[snafu(display(
        "memory quota exceeded: session {session_key} has {current} entries (max {max})"
    ))]
    MemoryQuotaExceeded {
        session_key: SessionKey,
        current:     usize,
        max:         usize,
    },

    /// Sandbox: access to a file path was denied.
    #[snafu(display("sandbox denied {operation} access to: {path}"))]
    SandboxAccessDenied {
        path:      String,
        operation: String,
    },

    /// Sandbox: path resolution failed (e.g., path traversal attempt).
    #[snafu(display("sandbox path error: {message}"))]
    SandboxPathError { message: String },

    /// Device already registered in the registry.
    #[snafu(display("device already registered: {id}"))]
    DeviceAlreadyRegistered { id: String },

    /// Device not found in the registry.
    #[snafu(display("device not found: {id}"))]
    DeviceNotFound { id: String },

    /// Device health check or shutdown failed.
    #[snafu(display("device error: {message}"))]
    Device { message: String },

    /// Catch-all: wraps any error with a descriptive message (via
    /// [`snafu::ResultExt::whatever_context`]).
    #[snafu(whatever, display("{message}"))]
    Whatever {
        message: String,
        #[snafu(source(from(Box<dyn std::error::Error + Send + Sync>, Some)))]
        source:  Option<Box<dyn std::error::Error + Send + Sync>>,
    },
}

/// Classify a provider error by HTTP status code and/or error message body.
///
/// Used by retry and fallback logic to decide whether to retry, fall back to
/// another model, or give up.
pub fn classify_provider_error(msg: &str, status_code: Option<u16>) -> KernelError {
    if matches!(status_code, Some(500 | 502 | 503 | 529)) {
        return KernelError::RetryableServer;
    }

    if is_context_window_error(msg) {
        KernelError::ContextWindow
    } else if is_retryable_server_error(msg) {
        KernelError::RetryableServer
    } else {
        KernelError::NonRetryable
    }
}

/// Whether the error is eligible for model fallback.
///
/// Context window errors and missing API key errors are NOT eligible because
/// switching models would not resolve them.
pub fn is_fallback_eligible(err: &KernelError) -> bool {
    !matches!(
        err,
        KernelError::ContextWindow | KernelError::ProviderNotConfigured { .. }
    )
}

pub fn is_retryable_provider_error(err: &KernelError) -> bool {
    match err {
        KernelError::Provider { message } => {
            matches!(
                classify_provider_error(message.as_ref(), None),
                KernelError::RetryableServer
            )
        }
        KernelError::RetryableServer => true,
        _ => false,
    }
}

// -- Private helpers ---------------------------------------------------------

/// Error patterns that indicate the context window has been exceeded.
const CONTEXT_WINDOW_PATTERNS: &[&str] = &[
    "context_length_exceeded",
    "max_tokens",
    "too many tokens",
    "request too large",
    "maximum context length",
    "context window",
    "token limit",
    "content_too_large",
    "request_too_large",
];

fn is_context_window_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    CONTEXT_WINDOW_PATTERNS
        .iter()
        .any(|pattern| lower.contains(pattern))
}

/// Error patterns that indicate a transient server error worth retrying.
const RETRYABLE_PATTERNS: &[&str] = &[
    "http 500",
    "http 502",
    "http 503",
    "http 529",
    "server_error",
    "internal server error",
    "overloaded",
    "bad gateway",
    "service unavailable",
    "the server had an error processing your request",
];

fn is_retryable_server_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    RETRYABLE_PATTERNS
        .iter()
        .any(|pattern| lower.contains(pattern))
}

pub type Result<T> = std::result::Result<T, KernelError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_eligible_for_retryable_server() {
        assert!(is_fallback_eligible(&KernelError::RetryableServer));
    }

    #[test]
    fn fallback_eligible_for_non_retryable() {
        assert!(is_fallback_eligible(&KernelError::NonRetryable));
    }

    #[test]
    fn fallback_eligible_for_other() {
        assert!(is_fallback_eligible(&KernelError::Other {
            message: "something went wrong".into(),
        }));
    }

    #[test]
    fn fallback_not_eligible_for_context_window() {
        assert!(!is_fallback_eligible(&KernelError::ContextWindow));
    }

    #[test]
    fn fallback_not_eligible_for_not_configured() {
        assert!(!is_fallback_eligible(&KernelError::ProviderNotConfigured {
            location: snafu::Location::new("test", 0, 0),
        }));
    }
}
