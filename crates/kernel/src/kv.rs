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

//! KV backend abstraction for cross-agent shared state.
//!
//! The default implementation uses an in-memory `DashMap` (volatile).
//! Production deployments can swap in a persistent backend (e.g., AgentFS).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

/// Shared reference to a [`KvBackend`] implementation.
pub type KvBackendRef = Arc<dyn KvBackend>;

/// Backend for the kernel's shared key-value store.
///
/// The default implementation uses an in-memory `DashMap` (volatile).
/// Production deployments can swap in a persistent backend (e.g., AgentFS).
#[async_trait]
pub trait KvBackend: Send + Sync {
    /// Get a value by key.
    async fn get(&self, key: &str) -> Option<Value>;

    /// Set a value. Creates or overwrites.
    async fn set(&self, key: &str, value: Value) -> anyhow::Result<()>;

    /// Delete a key.
    async fn delete(&self, key: &str) -> anyhow::Result<()>;

    /// List all keys with a given prefix.
    async fn list_prefix(&self, prefix: &str) -> Vec<(String, Value)>;

    /// Check whether a key exists without retrieving the value.
    async fn contains_key(&self, key: &str) -> bool { self.get(key).await.is_some() }

    /// Count keys with a given prefix.
    async fn count_prefix(&self, prefix: &str) -> usize { self.list_prefix(prefix).await.len() }
}
