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

use std::collections::HashMap;

use bon::Builder;
use serde::{Serialize, de::DeserializeOwned};
use snafu::ResultExt;
use sqlx::{PgPool, Postgres, QueryBuilder};
use tracing::info;
use uuid::Uuid;

use crate::err::*;

/// Key-value store backed by PostgreSQL
///
/// All values are serialized to JSON before storage
#[derive(Clone)]
pub struct KVStore {
    pool: PgPool,
}

impl KVStore {
    /// Create a new KV store from a PostgreSQL pool
    pub(crate) fn new(pool: PgPool) -> Self { Self { pool } }

    /// Set a key-value pair
    ///
    /// The value will be serialized to JSON before storage
    ///
    /// # Arguments
    /// * `key` - The key to store
    /// * `value` - The value to store (must implement Serialize)
    pub async fn set<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        let value_json = serde_json::to_string(value).context(CodecSnafu)?;

        sqlx::query(
            "INSERT INTO kv_table (key, value) VALUES ($1, $2) ON CONFLICT (key) DO UPDATE SET \
             value = EXCLUDED.value",
        )
        .bind(key)
        .bind(value_json)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get a value by key
    ///
    /// Returns `None` if the key does not exist
    /// The value will be deserialized from JSON
    ///
    /// # Arguments
    /// * `key` - The key to retrieve
    pub async fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value FROM kv_table WHERE key = $1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some((value_json,)) => {
                let value = serde_json::from_str(&value_json).context(CodecSnafu)?;
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    /// Remove a key-value pair
    ///
    /// # Arguments
    /// * `key` - The key to remove
    pub async fn remove(&self, key: &str) -> Result<()> {
        sqlx::query("DELETE FROM kv_table WHERE key = $1")
            .bind(key)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Batch set multiple key-value pairs
    ///
    /// All operations are performed within a single transaction for atomicity.
    /// If any operation fails, all changes will be rolled back.
    ///
    /// Optimized implementation:
    /// - Pre-serializes all values before starting the transaction
    /// - Uses batch SQL INSERT for better performance
    ///
    /// # Arguments
    /// * `pairs` - An iterator of (key, value) tuples to store
    ///
    /// # Example
    /// ```ignore
    /// let pairs = vec![
    ///     ("key1", "value1"),
    ///     ("key2", "value2"),
    ///     ("key3", "value3"),
    /// ];
    /// kv.batch_set(pairs).await?;
    /// ```
    pub async fn batch_set<T, I>(&self, pairs: I) -> Result<()>
    where
        T: Serialize,
        I: IntoIterator<Item = (String, T)>,
    {
        // Step 1: Pre-serialize all values (CPU-intensive, no await)
        let serialized_pairs: Vec<(String, String)> = pairs
            .into_iter()
            .map(|(key, value)| {
                let value_json = serde_json::to_string(&value).context(CodecSnafu)?;
                Ok((key, value_json))
            })
            .collect::<Result<Vec<_>>>()?;

        if serialized_pairs.is_empty() {
            return Ok(());
        }

        // Step 2: Execute batch insert in a single transaction
        let mut tx = self.pool.begin().await?;

        let mut builder = QueryBuilder::<Postgres>::new("INSERT INTO kv_table (key, value) ");
        builder.push_values(serialized_pairs.iter(), |mut row, (key, value_json)| {
            row.push_bind(key).push_bind(value_json);
        });
        builder.push(" ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value");
        builder.build().execute(&mut *tx).await?;
        tx.commit().await?;

        Ok(())
    }

    /// Batch get values for multiple keys
    ///
    /// Returns a HashMap containing only the keys that exist in the store.
    /// Keys that don't exist will not be present in the result.
    ///
    /// # Arguments
    /// * `keys` - An iterator of keys to retrieve
    ///
    /// # Example
    /// ```ignore
    /// let keys = vec!["key1", "key2", "key3"];
    /// let results: HashMap<String, String> = kv.batch_get(keys).await?;
    /// // results will only contain entries for keys that exist
    /// ```
    pub async fn batch_get<T, I>(&self, keys: I) -> Result<HashMap<String, T>>
    where
        T: DeserializeOwned,
        I: IntoIterator<Item = String>,
    {
        let keys: Vec<String> = keys.into_iter().collect();
        if keys.is_empty() {
            return Ok(HashMap::new());
        }

        let rows = sqlx::query_as::<_, (String, String)>(
            "SELECT key, value FROM kv_table WHERE key = ANY($1)",
        )
        .bind(&keys[..])
        .fetch_all(&self.pool)
        .await?;

        let mut result = HashMap::new();
        for (key, value_json) in rows {
            let value = serde_json::from_str(&value_json).context(CodecSnafu)?;
            result.insert(key, value);
        }

        Ok(result)
    }

    /// Batch get values for multiple keys, preserving order
    ///
    /// Returns a Vec of Options in the same order as the input keys.
    /// Keys that don't exist will have `None` at their position.
    ///
    /// # Arguments
    /// * `keys` - An iterator of keys to retrieve
    ///
    /// # Example
    /// ```ignore
    /// let keys = vec!["key1", "key2", "key3"];
    /// let results: Vec<Option<String>> = kv.batch_get_ordered(keys).await?;
    /// // results[0] corresponds to "key1", results[1] to "key2", etc.
    /// ```
    pub async fn batch_get_ordered<T, I>(&self, keys: I) -> Result<Vec<Option<T>>>
    where
        T: DeserializeOwned,
        I: IntoIterator<Item = String>,
    {
        let keys: Vec<String> = keys.into_iter().collect();
        if keys.is_empty() {
            return Ok(Vec::new());
        }

        // Get all values as a HashMap first
        let mut values = self.batch_get::<T, _>(keys.clone()).await?;

        // Map back to ordered Vec, removing values from HashMap
        let result = keys.into_iter().map(|key| values.remove(&key)).collect();

        Ok(result)
    }
}

#[derive(Clone, Debug)]
pub enum IdType {
    /// A new ID was generated and stored
    New(String),
    /// The key already existed
    Existing {
        /// The value that was already stored
        previous_value: String,
        /// The current/new value
        new_value:      String,
    },
}

/// Request for batch_get_or_init_keys
#[derive(Clone, Debug, Builder)]
#[builder(on(String, into))]
pub struct KeyRequest {
    /// The key to retrieve or initialize
    pub key:   String,
    /// If true, force update the key even if it exists
    #[builder(default = false)]
    pub force: bool,
}

#[async_trait::async_trait]
pub trait KVStoreExt {
    /// Get an existing key or initialize it with a new UUID if it doesn't exist
    ///
    /// # Arguments
    /// * `key` - The key to retrieve or initialize
    ///
    /// # Returns
    /// * `IdType::Existing(id)` if the key already exists
    /// * `IdType::New(id)` if a new UUID was generated and stored
    async fn get_or_init_key(&self, key: &str) -> Result<IdType>;

    /// Batch get or initialize multiple keys with UUIDs
    ///
    /// For each key:
    /// - If it exists and `force` is false, returns `IdType::Existing` with
    ///   same previous/new value
    /// - If it exists and `force` is true, generates a new UUID, updates the
    ///   key, and returns `IdType::Existing` with different previous/new values
    /// - If it doesn't exist, generates a new UUID and returns
    ///   `IdType::New(id)`
    ///
    /// All new/updated keys are inserted in a single transaction for atomicity.
    ///
    /// # Arguments
    /// * `keys` - An iterator of `KeyRequest` containing key and force flag
    ///
    /// # Returns
    /// A HashMap mapping each key to its IdType
    async fn batch_get_or_init_keys<I>(&self, keys: I) -> Result<HashMap<String, IdType>>
    where
        I: IntoIterator<Item = KeyRequest> + Send;
}

#[async_trait::async_trait]
impl KVStoreExt for KVStore {
    async fn get_or_init_key(&self, key: &str) -> Result<IdType> {
        if let Some(v) = self.get::<String>(key).await? {
            return Ok(IdType::Existing {
                previous_value: v.clone(),
                new_value:      v,
            });
        }

        let id = Uuid::new_v4().to_string();
        self.set(key, &id).await?;

        Ok(IdType::New(id))
    }

    async fn batch_get_or_init_keys<I>(&self, keys: I) -> Result<HashMap<String, IdType>>
    where
        I: IntoIterator<Item = KeyRequest> + Send,
    {
        let requests: Vec<KeyRequest> = keys.into_iter().collect();
        if requests.is_empty() {
            return Ok(HashMap::new());
        }

        // First, batch get existing keys
        let key_strings: Vec<String> = requests.iter().map(|r| r.key.clone()).collect();
        let existing = self.batch_get::<String, _>(key_strings).await?;

        // Identify keys that need initialization or update
        let mut write_pairs = Vec::new();
        let mut result = HashMap::new();

        for req in &requests {
            if let Some(previous_value) = existing.get(&req.key) {
                if req.force {
                    // Force update: generate new ID
                    let new_id = Uuid::new_v4().to_string();
                    write_pairs.push((req.key.clone(), new_id.clone()));
                    info!(
                        "Force updating identifier key '{}': {} -> {}",
                        req.key, previous_value, new_id
                    );
                    result.insert(
                        req.key.clone(),
                        IdType::Existing {
                            previous_value: previous_value.clone(),
                            new_value:      new_id,
                        },
                    );
                } else {
                    // No force: keep existing value
                    info!(
                        "Found existing identifier key '{}': {}",
                        req.key, previous_value
                    );
                    result.insert(
                        req.key.clone(),
                        IdType::Existing {
                            previous_value: previous_value.clone(),
                            new_value:      previous_value.clone(),
                        },
                    );
                }
            } else {
                let new_id = Uuid::new_v4().to_string();
                write_pairs.push((req.key.clone(), new_id.clone()));
                info!("Initialized new identifier key '{}': {}", req.key, &new_id);
                result.insert(req.key.clone(), IdType::New(new_id));
            }
        }

        // Batch set new/updated keys in a single transaction
        if !write_pairs.is_empty() {
            self.batch_set(write_pairs).await?;
        }

        Ok(result)
    }
}
