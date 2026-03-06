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

//! Knowledge service — bundles all knowledge layer dependencies.

use std::sync::Arc;

use sqlx::SqlitePool;

use super::{EmbeddingService, KnowledgeConfig};

/// Bundles the knowledge layer's runtime dependencies into a single handle
/// that can be shared across the kernel.
pub struct KnowledgeService {
    pub pool: SqlitePool,
    pub embedding_svc: Arc<EmbeddingService>,
    pub config: KnowledgeConfig,
}

/// Shared reference to the knowledge service.
pub type KnowledgeServiceRef = Arc<KnowledgeService>;
