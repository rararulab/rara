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

//! Unified memory layer: mem0 (facts) + Memos (notes) + Hindsight (4-network recall).
//!
//! # Architecture
//!
//! Three external services replace the previous homegrown PG + Chroma engine:
//!
//! | Service    | Role                        | API        |
//! |------------|-----------------------------|------------|
//! | **mem0**   | Structured fact management  | REST v1    |
//! | **Memos**  | Markdown note storage       | REST v1    |
//! | **Hindsight** | 4-network retain/recall/reflect | REST v1 |
//!
//! [`MemoryManager`] provides a unified facade that fans out to all three
//! backends and merges search results via Reciprocal Rank Fusion.

pub mod error;
pub mod fusion;
pub mod hindsight_client;
pub mod manager;
pub mod mem0_client;
pub mod memos_client;

pub use error::{MemoryError, MemoryResult};
pub use hindsight_client::HindsightClient;
pub use manager::{MemoryManager, MemorySource, SearchResult};
pub use mem0_client::{Mem0Client, Mem0Memory};
pub use memos_client::{MemosClient, MemoEntry};
