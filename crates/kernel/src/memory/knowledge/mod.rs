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

//! Knowledge Layer — structured long-term memory with embedding search.
//!
//! Three layers:
//! 1. **Tape** (existing, untouched) — raw conversation log
//! 2. **Memory Items** (SQLite + usearch) — extracted facts, preferences, events
//! 3. **Category Files** (markdown on disk) — organized knowledge summaries

pub mod categories;
pub mod config;
pub mod embedding;
pub mod extractor;
pub mod items;
pub mod service;
pub mod tool;

pub use config::KnowledgeConfig;
pub use embedding::EmbeddingService;
pub use service::{KnowledgeService, KnowledgeServiceRef};
pub use tool::MemoryTool;
