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
