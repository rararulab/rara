pub mod types;

pub use types::{PromptEntry, PromptError, PromptSpec};

/// Async trait for prompt storage and retrieval.
#[async_trait::async_trait]
pub trait PromptRepo: Send + Sync + 'static {
    /// Get a single prompt by name. Returns `None` if not registered.
    async fn get(&self, name: &str) -> Option<PromptEntry>;

    /// List all registered prompts.
    async fn list(&self) -> Vec<PromptEntry>;

    /// Update a prompt's content (writes to backing store + refreshes cache).
    async fn update(&self, name: &str, content: &str) -> Result<PromptEntry, PromptError>;

    /// Reset a prompt to its compiled-in default content.
    async fn reset(&self, name: &str) -> Result<PromptEntry, PromptError>;
}
