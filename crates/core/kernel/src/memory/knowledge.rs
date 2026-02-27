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

//! Knowledge memory layer — persistent document and note storage.
//!
//! Semantically equivalent to Memos: human-readable Markdown notes
//! with tag-based organisation.

use uuid::Uuid;

use super::{
    error::Result,
    types::{KnowledgeNote, MemoryContext, Scope},
};

/// Persistent knowledge note storage.
#[async_trait::async_trait]
pub trait KnowledgeMemory: Send + Sync {
    /// Write a knowledge note with optional tags.
    async fn write(
        &self,
        ctx: &MemoryContext,
        scope: Scope,
        content: &str,
        tags: &[&str],
    ) -> Result<KnowledgeNote>;

    /// Read a single note by ID.
    async fn read(
        &self,
        ctx: &MemoryContext,
        scope: Scope,
        id: Uuid,
    ) -> Result<Option<KnowledgeNote>>;

    /// List notes, optionally filtered by tags.
    async fn list(
        &self,
        ctx: &MemoryContext,
        scope: Scope,
        tags: &[&str],
        limit: usize,
    ) -> Result<Vec<KnowledgeNote>>;

    /// Delete a single note.
    async fn delete(&self, ctx: &MemoryContext, scope: Scope, id: Uuid) -> Result<()>;
}
