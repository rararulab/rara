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

//! Background worker that periodically syncs the memory index.
//!
//! Runs on a cron schedule (default every 5 minutes) so that newly written
//! markdown files in the memory directory are indexed into PostgreSQL and
//! Chroma without blocking search requests.

use async_trait::async_trait;
use common_worker::{FallibleWorker, WorkError, WorkResult, WorkerContext};
use tracing::info;

use crate::worker_state::AppState;

/// Periodically calls `MemoryManager::sync()` to index new or changed
/// markdown files in the memory directory.
pub struct MemorySyncWorker;

#[async_trait]
impl FallibleWorker<AppState> for MemorySyncWorker {
    async fn work(&mut self, ctx: WorkerContext<AppState>) -> WorkResult {
        let state = ctx.state();

        let stats = state
            .memory_manager
            .sync()
            .await
            .map_err(|e| WorkError::transient(format!("memory sync failed: {e}")))?;

        info!(
            indexed = stats.indexed_files,
            deleted = stats.deleted_files,
            chunks = stats.total_chunks,
            "memory sync completed"
        );

        Ok(())
    }
}
