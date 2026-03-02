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

//! Learning memory layer — experience retention, recall, and reflection.
//!
//! Semantically equivalent to Hindsight's 4-network model:
//! retain experiences, recall relevant ones, and reflect for synthesis.

use super::{
    error::Result,
    types::{MemoryContext, RecallEntry, Scope},
};

/// Experience-based learning memory.
#[async_trait::async_trait]
pub trait LearningMemory: Send + Sync {
    /// Store content into long-term experience memory.
    async fn retain(&self, ctx: &MemoryContext, scope: Scope, content: &str) -> Result<()>;

    /// Semantically recall relevant experiences.
    async fn recall(
        &self,
        ctx: &MemoryContext,
        scope: Scope,
        query: &str,
        limit: usize,
    ) -> Result<Vec<RecallEntry>>;

    /// Deep reflection — synthesise an answer by reasoning across all
    /// stored experiences.
    async fn reflect(&self, ctx: &MemoryContext, scope: Scope, query: &str) -> Result<String>;
}
