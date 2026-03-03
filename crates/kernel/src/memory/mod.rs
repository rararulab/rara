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

//! # Memory
//!
//! Pure trait abstractions for a 3-layer x 3-scope memory system.
//!
//! ## Layers
//!
//! | Trait              | Semantics    | Analogous to |
//! |--------------------|-------------|--------------|
//! | [`StateMemory`]    | Structured fact CRUD | mem0 |
//! | [`KnowledgeMemory`]| Persistent notes     | Memos |
//! | [`LearningMemory`] | Experience learning  | Hindsight |
//!
//! ## Scopes
//!
//! Each trait method takes a [`Scope`] to partition visibility:
//!
//! - [`Scope::Global`] — shared across all agents
//! - [`Scope::Team`] — shared within a team / project
//! - [`Scope::Agent`] — private to the calling agent
//!
//! ## Identity
//!
//! [`MemoryContext`] carries the caller's identity (`user_id`, `agent_id`,
//! `session_id`) and is passed to every trait method.

pub mod compaction;
pub mod error;
pub mod knowledge;
pub mod learning;
pub mod state;
pub mod types;

use std::sync::Arc;

pub use error::{MemoryError, Result};
pub use knowledge::KnowledgeMemory;
pub use learning::LearningMemory;
pub use state::StateMemory;
pub use types::*;

pub type MemoryRef = Arc<dyn Memory>;

/// Unified memory trait — combines all three memory layers.
///
/// Any type implementing `StateMemory + KnowledgeMemory + LearningMemory`
/// automatically implements `Memory` via blanket impl.
/// Consumers hold `Arc<dyn Memory>` for polymorphic access to all layers.
pub trait Memory: StateMemory + KnowledgeMemory + LearningMemory {}
impl<T: StateMemory + KnowledgeMemory + LearningMemory> Memory for T {}

// ---------------------------------------------------------------------------
// NoopMemory
// ---------------------------------------------------------------------------

mod noop {
    use async_trait::async_trait;
    use uuid::Uuid;

    use super::{
        KnowledgeMemory, LearningMemory, Result as MemResult, StateMemory,
        types::*,
    };

    /// A memory implementation that does nothing — all ops succeed with empty
    /// results.
    pub struct NoopMemory;

    #[async_trait]
    impl StateMemory for NoopMemory {
        async fn add(
            &self,
            _ctx: &MemoryContext,
            _scope: Scope,
            _messages: Vec<Message>,
        ) -> MemResult<Vec<StateEvent>> {
            Ok(vec![])
        }

        async fn search(
            &self,
            _ctx: &MemoryContext,
            _scope: Scope,
            _query: &str,
            _limit: usize,
        ) -> MemResult<Vec<StateFact>> {
            Ok(vec![])
        }

        async fn get(
            &self,
            _ctx: &MemoryContext,
            _scope: Scope,
            _id: Uuid,
        ) -> MemResult<Option<StateFact>> {
            Ok(None)
        }

        async fn get_all(
            &self,
            _ctx: &MemoryContext,
            _scope: Scope,
            _limit: usize,
        ) -> MemResult<Vec<StateFact>> {
            Ok(vec![])
        }

        async fn update(
            &self,
            _ctx: &MemoryContext,
            _scope: Scope,
            _id: Uuid,
            _data: &str,
        ) -> MemResult<()> {
            Ok(())
        }

        async fn delete(&self, _ctx: &MemoryContext, _scope: Scope, _id: Uuid) -> MemResult<()> {
            Ok(())
        }

        async fn delete_all(&self, _ctx: &MemoryContext, _scope: Scope) -> MemResult<()> { Ok(()) }

        async fn history(
            &self,
            _ctx: &MemoryContext,
            _scope: Scope,
            _id: Uuid,
        ) -> MemResult<Vec<StateHistory>> {
            Ok(vec![])
        }
    }

    #[async_trait]
    impl KnowledgeMemory for NoopMemory {
        async fn write(
            &self,
            _ctx: &MemoryContext,
            _scope: Scope,
            _content: &str,
            _tags: &[&str],
        ) -> MemResult<KnowledgeNote> {
            Ok(KnowledgeNote {
                id:         Uuid::new_v4(),
                content:    String::new(),
                tags:       vec![],
                created_at: jiff::Timestamp::now(),
                updated_at: jiff::Timestamp::now(),
            })
        }

        async fn read(
            &self,
            _ctx: &MemoryContext,
            _scope: Scope,
            _id: Uuid,
        ) -> MemResult<Option<KnowledgeNote>> {
            Ok(None)
        }

        async fn list(
            &self,
            _ctx: &MemoryContext,
            _scope: Scope,
            _tags: &[&str],
            _limit: usize,
        ) -> MemResult<Vec<KnowledgeNote>> {
            Ok(vec![])
        }

        async fn delete(&self, _ctx: &MemoryContext, _scope: Scope, _id: Uuid) -> MemResult<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl LearningMemory for NoopMemory {
        async fn retain(&self, _ctx: &MemoryContext, _scope: Scope, _content: &str) -> MemResult<()> {
            Ok(())
        }

        async fn recall(
            &self,
            _ctx: &MemoryContext,
            _scope: Scope,
            _query: &str,
            _limit: usize,
        ) -> MemResult<Vec<RecallEntry>> {
            Ok(vec![])
        }

        async fn reflect(
            &self,
            _ctx: &MemoryContext,
            _scope: Scope,
            _query: &str,
        ) -> MemResult<String> {
            Ok(String::new())
        }
    }
}

pub use noop::NoopMemory;
