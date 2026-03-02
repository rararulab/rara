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

//! Default in-memory KV backend using DashMap.

use async_trait::async_trait;
use dashmap::DashMap;
use serde_json::Value;

use crate::kv::KvBackend;

/// Volatile in-memory KV backend.
pub struct DashMapKv {
    map: DashMap<String, Value>,
}

impl DashMapKv {
    pub fn new() -> Self {
        Self {
            map: DashMap::new(),
        }
    }
}

impl Default for DashMapKv {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl KvBackend for DashMapKv {
    async fn get(&self, key: &str) -> Option<Value> { self.map.get(key).map(|v| v.value().clone()) }

    async fn set(&self, key: &str, value: Value) -> anyhow::Result<()> {
        self.map.insert(key.to_owned(), value);
        Ok(())
    }

    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        self.map.remove(key);
        Ok(())
    }

    async fn list_prefix(&self, prefix: &str) -> Vec<(String, Value)> {
        self.map
            .iter()
            .filter(|entry| entry.key().starts_with(prefix))
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    async fn contains_key(&self, key: &str) -> bool { self.map.contains_key(key) }

    async fn count_prefix(&self, prefix: &str) -> usize {
        self.map
            .iter()
            .filter(|entry| entry.key().starts_with(prefix))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_basic_crud() {
        let kv = DashMapKv::new();

        // Initially empty
        assert!(kv.get("key1").await.is_none());

        // Set and get
        kv.set("key1", serde_json::json!("value1")).await.unwrap();
        assert_eq!(kv.get("key1").await, Some(serde_json::json!("value1")));

        // Overwrite
        kv.set("key1", serde_json::json!("value2")).await.unwrap();
        assert_eq!(kv.get("key1").await, Some(serde_json::json!("value2")));

        // Delete
        kv.delete("key1").await.unwrap();
        assert!(kv.get("key1").await.is_none());
    }

    #[tokio::test]
    async fn test_list_prefix() {
        let kv = DashMapKv::new();

        kv.set("agent:1:foo", serde_json::json!("a")).await.unwrap();
        kv.set("agent:1:bar", serde_json::json!("b")).await.unwrap();
        kv.set("agent:2:foo", serde_json::json!("c")).await.unwrap();

        let results = kv.list_prefix("agent:1:").await;
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_contains_key() {
        let kv = DashMapKv::new();

        assert!(!kv.contains_key("missing").await);
        kv.set("present", serde_json::json!(42)).await.unwrap();
        assert!(kv.contains_key("present").await);
    }

    #[tokio::test]
    async fn test_count_prefix() {
        let kv = DashMapKv::new();

        kv.set("team:a:x", serde_json::json!(1)).await.unwrap();
        kv.set("team:a:y", serde_json::json!(2)).await.unwrap();
        kv.set("team:b:z", serde_json::json!(3)).await.unwrap();

        assert_eq!(kv.count_prefix("team:a:").await, 2);
        assert_eq!(kv.count_prefix("team:b:").await, 1);
        assert_eq!(kv.count_prefix("team:").await, 3);
    }
}
