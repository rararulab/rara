mod builtin;
pub mod types;

pub use builtin::{BuiltinPromptRepo, all_builtin_prompts};
pub use types::{PromptEntry, PromptError, PromptSpec};

/// Async trait for prompt retrieval (read-only).
#[async_trait::async_trait]
pub trait PromptRepo: Send + Sync + 'static {
    /// Get a single prompt by name. Returns `None` if not registered.
    async fn get(&self, name: &str) -> Option<PromptEntry>;

    /// List all registered prompts.
    async fn list(&self) -> Vec<PromptEntry>;
}
