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

//! Centralized model configuration trait.
//!
//! Consumers read model assignments through [`ModelRepo`] instead of
//! accessing scattered settings fields directly. The trait is intentionally
//! simple: keys map to model identifiers and a global fallback list
//! provides resilience when the primary model is unavailable.

/// Well-known model key constants used across the platform.
pub mod model_keys {
    pub const DEFAULT: &str = "default";
    pub const CHAT: &str = "chat";
    pub const JOB: &str = "job";
    pub const PROACTIVE: &str = "proactive";
    pub const SCHEDULED: &str = "scheduled";
}

/// A single key-model mapping entry.
pub struct ModelEntry {
    pub key:   String,
    pub model: String,
}

/// Errors produced by [`ModelRepo`] operations.
#[derive(Debug, snafu::Snafu)]
pub enum ModelRepoError {
    #[snafu(display("persistence error: {message}"))]
    Persistence { message: String },
}

/// Unified trait for reading and writing model configuration.
///
/// Implementations are expected to be backed by runtime settings so
/// that changes take effect immediately without restart.
#[async_trait::async_trait]
pub trait ModelRepo: Send + Sync + 'static {
    /// Get the model for the given key.
    ///
    /// Returns `None` if no model is configured for the key (or the
    /// `"default"` fallback key). Callers must handle the missing-model
    /// case explicitly.
    async fn get(&self, key: &str) -> Option<String>;

    /// Assign a model to a key.
    async fn set(&self, key: &str, model: &str) -> Result<(), ModelRepoError>;

    /// Remove a key-model mapping.
    async fn remove(&self, key: &str) -> Result<(), ModelRepoError>;

    /// List all key-model mappings.
    async fn list(&self) -> Vec<ModelEntry>;

    /// Return the global fallback model list (tried in order when primary
    /// fails).
    async fn fallback_models(&self) -> Vec<String>;

    /// Replace the global fallback model list.
    async fn set_fallback_models(&self, models: Vec<String>) -> Result<(), ModelRepoError>;
}
