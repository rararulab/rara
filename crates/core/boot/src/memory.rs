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

//! Memory manager and recall strategy engine factory functions.

use std::sync::Arc;

use tracing::info;

/// Create a [`MemoryManager`](rara_memory::MemoryManager) from service URLs.
pub fn init_memory_manager(
    mem0_base_url: String,
    memos_base_url: String,
    memos_token: String,
    hindsight_base_url: String,
    hindsight_bank_id: String,
) -> Arc<rara_memory::MemoryManager> {
    info!("mem0 using direct connection to {}", mem0_base_url);
    let mem0 = rara_memory::Mem0Client::new(mem0_base_url);
    let memos = rara_memory::MemosClient::new(memos_base_url, memos_token);
    let hindsight = rara_memory::HindsightClient::new(hindsight_base_url, hindsight_bank_id);
    let manager = rara_memory::MemoryManager::new(mem0, memos, hindsight, "default".to_owned());
    info!("memory manager initialized");
    Arc::new(manager)
}

/// Create a [`RecallStrategyEngine`](rara_memory::RecallStrategyEngine) with
/// default rules.
pub fn init_recall_engine() -> Arc<rara_memory::RecallStrategyEngine> {
    let engine =
        rara_memory::RecallStrategyEngine::new(rara_memory::recall_engine::default_rules());
    info!("Recall strategy engine initialized with default rules");
    Arc::new(engine)
}
