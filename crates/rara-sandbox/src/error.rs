//! Typed errors for the sandbox crate.

use snafu::prelude::*;

/// Errors returned by [`Sandbox`](crate::Sandbox) operations.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum SandboxError {
    /// Failure bubbled up from the boxlite runtime.
    ///
    /// Covers VM provisioning, rootfs pulls, exec channel setup, and any
    /// other failure reported by [`boxlite`]. Inspect the wrapped error for
    /// the precise cause; boxlite does not (yet) provide a stable way to
    /// categorise these programmatically.
    #[snafu(display("boxlite runtime error: {source}"))]
    Boxlite { source: boxlite::BoxliteError },

    /// The requested stdout stream was already consumed.
    ///
    /// boxlite's [`Execution::stdout`](boxlite::Execution::stdout) returns
    /// the stream handle by move on the first call and `None` thereafter.
    /// Seeing this error means the caller (or a prior wrapper) already took
    /// it; create a fresh [`ExecRequest`](crate::ExecRequest) instead of
    /// trying to re-consume the same execution.
    #[snafu(display("sandbox execution produced no stdout stream"))]
    MissingStdout,
}

/// Convenience alias for results produced by this crate.
pub type Result<T> = std::result::Result<T, SandboxError>;
