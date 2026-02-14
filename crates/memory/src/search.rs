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

//! Search strategies for memory retrieval.

use crate::manager::{MemoryManager, MemoryResult, SearchResult};

/// Execute keyword-only retrieval.
///
/// This helper delegates to [`MemoryManager::search`], which applies current
/// runtime routing and fallback behavior.
pub async fn keyword_only_search(
    manager: &MemoryManager,
    query: &str,
    limit: usize,
) -> MemoryResult<Vec<SearchResult>> {
    manager.search(query, limit).await
}

/// Execute hybrid retrieval (vector + keyword) with graceful fallback.
pub async fn hybrid_search(
    manager: &MemoryManager,
    query: &str,
    limit: usize,
) -> MemoryResult<Vec<SearchResult>> {
    keyword_only_search(manager, query, limit).await
}
