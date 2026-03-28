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

use std::collections::HashMap;

use bon::Builder;
use serde::{Serialize, de::DeserializeOwned};
use snafu::ResultExt;
use sqlx::{QueryBuilder, Sqlite, SqlitePool};
use tracing::info;
use uuid::Uuid;

use crate::error::*;

/// Key-value store backed by SQLite.
///
/// All values are serialized to JSON before storage.
#[derive(Clone)]
pub struct KVStore {
    pool: SqlitePool,
}

impl KVStore {
    /// Create a new KV store from a SQLite pool.
    pub(crate) fn new(pool: SqlitePool) -> Self { Self { pool } }

    /// Set a key-value pair.
    ///
    /// The value will be serialized to JSON before storage.
    pub async fn set<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        let value_json = serde_json::to_string(value).context(CodecSnafu)?;

        sqlx::query(
            "INSERT INTO kv_table (key, value) VALUES (?, ?) ON CONFLICT (key) DO UPDATE SET \
             value = EXCLUDED.value",
        )
        .bind(key)
        .bind(value_json)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get a value by key.
    ///
    /// Returns `None` if the key does not exist.
    pub async fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value FROM kv_table WHERE key = ?")
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

    /// Remove a key-value pair.
    pub async fn remove(&self, key: &str) -> Result<()> {
        sqlx::query("DELETE FROM kv_table WHERE key = ?")
            .bind(key)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Batch set multiple key-value pairs.
    ///
    /// All operations are performed within a single transaction for atomicity.
    pub async fn batch_set<T, I>(&self, pairs: I) -> Result<()>
    where
        T: Serialize,
        I: IntoIterator<Item = (String, T)>,
    {
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

        let mut tx = self.pool.begin().await?;

        let mut builder = QueryBuilder::<Sqlite>::new("INSERT INTO kv_table (key, value) ");
        builder.push_values(serialized_pairs.iter(), |mut row, (key, value_json)| {
            row.push_bind(key).push_bind(value_json);
        });
        builder.push(" ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value");
        builder.build().execute(&mut *tx).await?;
        tx.commit().await?;

        Ok(())
    }

    /// Batch get values for multiple keys.
    ///
    /// Returns a HashMap containing only the keys that exist in the store.
    pub async fn batch_get<T, I>(&self, keys: I) -> Result<HashMap<String, T>>
    where
        T: DeserializeOwned,
        I: IntoIterator<Item = String>,
    {
        let keys: Vec<String> = keys.into_iter().collect();
        if keys.is_empty() {
            return Ok(HashMap::new());
        }

        // SQLite does not support ANY($1) with array binding.
        // Build a dynamic IN (?, ?, ...) clause instead.
        let mut builder =
            QueryBuilder::<Sqlite>::new("SELECT key, value FROM kv_table WHERE key IN (");
        let mut separated = builder.separated(", ");
        for key in &keys {
            separated.push_bind(key);
        }
        separated.push_unseparated(")");

        let rows: Vec<(String, String)> = builder.build_query_as().fetch_all(&self.pool).await?;

        let mut result = HashMap::new();
        for (key, value_json) in rows {
            let value = serde_json::from_str(&value_json).context(CodecSnafu)?;
            result.insert(key, value);
        }

        Ok(result)
    }

    /// Batch get values for multiple keys, preserving order.
    ///
    /// Returns a Vec of Options in the same order as the input keys.
    pub async fn batch_get_ordered<T, I>(&self, keys: I) -> Result<Vec<Option<T>>>
    where
        T: DeserializeOwned,
        I: IntoIterator<Item = String>,
    {
        let keys: Vec<String> = keys.into_iter().collect();
        if keys.is_empty() {
            return Ok(Vec::new());
        }

        let mut values = self.batch_get::<T, _>(keys.clone()).await?;
        let result = keys.into_iter().map(|key| values.remove(&key)).collect();

        Ok(result)
    }
}

#[derive(Clone, Debug)]
pub enum IdType {
    /// A new ID was generated and stored.
    New(String),
    /// The key already existed.
    Existing {
        previous_value: String,
        new_value:      String,
    },
}

/// Request for batch_get_or_init_keys.
#[derive(Clone, Debug, Builder)]
#[builder(on(String, into))]
pub struct KeyRequest {
    /// The key to retrieve or initialize.
    pub key:   String,
    /// If true, force update the key even if it exists.
    #[builder(default = false)]
    pub force: bool,
}

#[async_trait::async_trait]
pub trait KVStoreExt {
    async fn get_or_init_key(&self, key: &str) -> Result<IdType>;

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

        let key_strings: Vec<String> = requests.iter().map(|r| r.key.clone()).collect();
        let existing = self.batch_get::<String, _>(key_strings).await?;

        let mut write_pairs = Vec::new();
        let mut result = HashMap::new();

        for req in &requests {
            if let Some(previous_value) = existing.get(&req.key) {
                if req.force {
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

        if !write_pairs.is_empty() {
            self.batch_set(write_pairs).await?;
        }

        Ok(result)
    }
}
