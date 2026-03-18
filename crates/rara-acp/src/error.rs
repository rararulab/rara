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
}
