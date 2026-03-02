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

//! State memory layer — structured fact extraction and retrieval.
//!
//! Semantically equivalent to mem0: automatic inference, deduplication,
//! and semantic search over structured facts.

use uuid::Uuid;

use super::{
    error::Result,
    types::{MemoryContext, Message, Scope, StateEvent, StateFact, StateHistory},
};

/// Structured fact memory — infer, store, search, and manage facts.
#[async_trait::async_trait]
pub trait StateMemory: Send + Sync {
    /// Extract facts from conversation messages and store them.
    ///
    /// The implementation may perform automatic inference and deduplication.
    async fn add(
        &self,
        ctx: &MemoryContext,
        scope: Scope,
        messages: Vec<Message>,
    ) -> Result<Vec<StateEvent>>;

    /// Semantic search over stored facts.
    async fn search(
        &self,
        ctx: &MemoryContext,
        scope: Scope,
        query: &str,
        limit: usize,
    ) -> Result<Vec<StateFact>>;

    /// Retrieve a single fact by ID.
    async fn get(&self, ctx: &MemoryContext, scope: Scope, id: Uuid) -> Result<Option<StateFact>>;

    /// List all facts within the given scope.
    async fn get_all(
        &self,
        ctx: &MemoryContext,
        scope: Scope,
        limit: usize,
    ) -> Result<Vec<StateFact>>;

    /// Update the content of a single fact.
    async fn update(&self, ctx: &MemoryContext, scope: Scope, id: Uuid, data: &str) -> Result<()>;

    /// Delete a single fact.
    async fn delete(&self, ctx: &MemoryContext, scope: Scope, id: Uuid) -> Result<()>;

    /// Delete all facts within the given scope.
    async fn delete_all(&self, ctx: &MemoryContext, scope: Scope) -> Result<()>;

    /// Retrieve the change history for a single fact.
    async fn history(
        &self,
        ctx: &MemoryContext,
        scope: Scope,
        id: Uuid,
    ) -> Result<Vec<StateHistory>>;
}
