//! Configuration types for sandbox creation and command execution.

use std::time::Duration;

use bon::Builder;
use serde::{Deserialize, Serialize};

/// Configuration describing how a [`Sandbox`](crate::Sandbox) should be
/// provisioned.
///
/// Field values are passed through to boxlite without interpretation — in
/// particular the rootfs image reference MUST already be resolvable by the
/// host's boxlite image store. `rara-sandbox` never supplies a default image
/// in Rust: the application layer is responsible for reading the image name
/// from YAML config and passing it in here.
///
/// The struct derives [`Deserialize`] so application-layer code may load it
/// straight from YAML, e.g.:
///
/// ```yaml
/// sandbox:
///   rootfs_image: "alpine:latest"
///   name: "my-agent-sandbox"
/// ```
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct SandboxConfig {
    /// OCI image reference passed to boxlite as
    /// [`RootfsSpec::Image`](boxlite::RootfsSpec::Image).
    pub rootfs_image: String,

    /// Optional human-readable box name. When `None`, boxlite generates one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Description of a single command to run inside a [`Sandbox`](crate::Sandbox).
///
/// Mirrors the subset of [`boxlite::BoxCommand`] that rara actually uses
/// today. Extra boxlite knobs (`tty`, `user`, `working_dir`) can be added here
/// when a concrete caller needs them — not before, to keep the API surface
/// minimal.
#[derive(Debug, Clone, Builder)]
pub struct ExecRequest {
    /// Executable to invoke inside the sandbox (e.g. `"echo"`, `"python"`).
    pub command: String,

    /// Arguments to pass, in order. Empty vec means no args.
    #[builder(default)]
    pub args: Vec<String>,

    /// Environment variables exported to the command.
    #[builder(default)]
    pub env: Vec<(String, String)>,

    /// Optional hard timeout enforced by boxlite.
    pub timeout: Option<Duration>,
}
