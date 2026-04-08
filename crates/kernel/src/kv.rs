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

//! Cross-agent shared key-value store backed by OpenDAL.
//!
//! Wraps an [`opendal::Operator`] and handles JSON serialization
//! transparently. The caller picks the backend (Memory, Fs, S3, …)
//! when constructing the kernel; the kernel itself only sees this
//! thin wrapper.

use opendal::Operator;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use snafu::prelude::*;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Typed errors for [`SharedKv`] operations.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum KvError {
    /// JSON serialization/deserialization failed.
    #[snafu(display("serialization failed: {source}"))]
    Serialize { source: serde_json::Error },

    /// Underlying OpenDAL storage operation failed.
    #[snafu(display("storage operation failed: {source}"))]
    Storage { source: opendal::Error },
}

/// Result alias for [`KvError`].
pub type Result<T> = std::result::Result<T, KvError>;

/// Cross-agent shared KV store backed by an [`opendal::Operator`].
pub struct SharedKv {
    op: Operator,
}

impl SharedKv {
    /// Create a new `SharedKv` around the given OpenDAL operator.
    pub fn new(op: Operator) -> Self { Self { op } }

    /// Create a volatile in-memory instance (tests / dev default).
    ///
    /// # Panics
    ///
    /// Never in practice — `opendal::services::Memory` construction is
    /// infallible. The `.expect()` guards against future opendal API changes.
    pub fn in_memory() -> Self {
        let op = Operator::new(opendal::services::Memory::default())
            .expect("opendal Memory operator is infallible")
            .finish();
        Self { op }
    }

    /// Get a JSON value by key. Returns `None` if the key does not exist.
    pub async fn get(&self, key: &str) -> Option<Value> {
        match self.op.read(key).await {
            Ok(buf) => serde_json::from_slice(&buf.to_vec()).ok(),
            Err(_) => None,
        }
    }

    /// Set a JSON value. Creates or overwrites.
    pub async fn set(&self, key: &str, value: Value) -> Result<()> {
        let bytes = serde_json::to_vec(&value).context(SerializeSnafu)?;
        self.op.write(key, bytes).await.context(StorageSnafu)?;
        Ok(())
    }

    /// Delete a key (no-op if absent).
    pub async fn delete(&self, key: &str) -> Result<()> {
        self.op.delete(key).await.context(StorageSnafu)?;
        Ok(())
    }

    /// Check whether a key exists.
    pub async fn contains_key(&self, key: &str) -> bool {
        self.op.exists(key).await.unwrap_or(false)
    }

    /// List all key-value pairs whose key starts with `prefix`.
    pub async fn list_prefix(&self, prefix: &str) -> Vec<(String, Value)> {
        let mut results = Vec::new();
        let entries = match self.op.list(prefix).await {
            Ok(entries) => entries,
            Err(_) => return results,
        };
        for entry in entries {
            let path = entry.path();
            if let Ok(buf) = self.op.read(path).await {
                if let Ok(val) = serde_json::from_slice::<Value>(&buf.to_vec()) {
                    results.push((path.to_owned(), val));
                }
            }
        }
        results
    }

    /// Count keys with a given prefix.
    pub async fn count_prefix(&self, prefix: &str) -> usize {
        match self.op.list(prefix).await {
            Ok(entries) => entries.len(),
            Err(_) => 0,
        }
    }
}

// ---------------------------------------------------------------------------
// KvScope
// ---------------------------------------------------------------------------

/// Visibility partition for KV shared memory operations.
///
/// Used by `ProcessHandle::shared_store` and `ProcessHandle::shared_recall`
/// to provide cross-agent data sharing with explicit scope control.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KvScope {
    /// Global scope — key stored as-is. Requires Root or Admin role.
    Global,
    /// Team scope — key prefixed with `"team:{name}:"`. Requires Root or
    /// Admin role.
    Team(String),
    /// Agent scope — key prefixed with `"agent:{agent_id}:"`. Regular agents
    /// can only access their own agent scope; Root/Admin can access any.
    Agent(uuid::Uuid),
}
