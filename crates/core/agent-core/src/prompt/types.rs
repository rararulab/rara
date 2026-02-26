/// Static specification for a registered prompt.
/// Includes the compiled-in default content via `include_str!()`.
#[derive(Debug, Clone)]
pub struct PromptSpec {
    /// Unique name, e.g. `"ai/job_fit.system.md"`.
    pub name:            &'static str,
    /// Human-readable description.
    pub description:     &'static str,
    /// Default content compiled into the binary.
    pub default_content: &'static str,
}

/// A resolved prompt entry with its current effective content.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PromptEntry {
    /// Unique name, e.g. `"ai/job_fit.system.md"`.
    pub name:        String,
    /// Human-readable description.
    pub description: String,
    /// Current effective content (compiled-in default).
    pub content:     String,
}

/// Errors produced by prompt operations.
#[derive(Debug, snafu::Snafu)]
#[snafu(visibility(pub))]
pub enum PromptError {
    #[snafu(display("prompt not found: {name}"))]
    NotFound { name: String },
}
