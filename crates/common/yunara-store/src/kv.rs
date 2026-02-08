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
use sqlx::SqlitePool;
use tracing::info;
use uuid::Uuid;

use crate::err::*;

/// Key-value store backed by SQLite
///
/// All values are serialized to JSON before storage
#[derive(Clone)]
pub struct KVStore {
    pool: SqlitePool,
}

impl KVStore {
    /// Create a new KV store from a SQLite pool
    pub(crate) fn new(pool: SqlitePool) -> Self { Self { pool } }

    /// Set a key-value pair
    ///
    /// The value will be serialized to JSON before storage
    ///
    /// # Arguments
    /// * `key` - The key to store
    /// * `value` - The value to store (must implement Serialize)
    pub async fn set<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        let value_json = serde_json::to_string(value).context(CodecSnafu)?;

        sqlx::query("INSERT OR REPLACE INTO kv_table (key, value) VALUES (?, ?)")
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

    /// Remove a key-value pair
    ///
    /// # Arguments
    /// * `key` - The key to remove
    pub async fn remove(&self, key: &str) -> Result<()> {
        sqlx::query("DELETE FROM kv_table WHERE key = ?")
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

        // Build batch INSERT statement
        // SQLite: INSERT OR REPLACE INTO kv_table (key, value) VALUES (?, ?), (?, ?),
        // ...
        let placeholders = serialized_pairs
            .iter()
            .map(|_| "(?, ?)")
            .collect::<Vec<_>>()
            .join(", ");

        let query_str = format!(
            "INSERT OR REPLACE INTO kv_table (key, value) VALUES {}",
            placeholders
        );

        let mut query = sqlx::query(&query_str);
        for (key, value_json) in &serialized_pairs {
            query = query.bind(key).bind(value_json);
        }

        query.execute(&mut *tx).await?;
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

        // Build IN clause placeholders
        let placeholders = keys.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        let query_str = format!(
            "SELECT key, value FROM kv_table WHERE key IN ({})",
            placeholders
        );

        let mut query = sqlx::query_as::<_, (String, String)>(&query_str);
        for key in &keys {
            query = query.bind(key);
        }

        let rows = query.fetch_all(&self.pool).await?;

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

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use tempfile::TempDir;

    use crate::{DatabaseConfig, db::DBStore};

    #[derive(Serialize, Deserialize, Clone, Eq, PartialEq, Debug)]
    struct Person {
        name: String,
        age:  i32,
    }

    #[tokio::test]
    async fn kv_store_test() {
        let tempdir = TempDir::new().unwrap();
        let db_path = tempdir.path().join("test.db");

        let config = DatabaseConfig {
            db_path,
            ..Default::default()
        };
        let db = DBStore::new(config).await.unwrap();
        let kv = db.kv_store();

        // Test string
        kv.set("str_key", &"hello".to_string()).await.unwrap();
        assert_eq!(kv.get::<String>("str_key").await.unwrap().unwrap(), "hello");
        assert_eq!(kv.get::<String>("nonexistent").await.unwrap(), None);

        // Test bool
        kv.set("bool_key", &true).await.unwrap();
        assert_eq!(kv.get::<bool>("bool_key").await.unwrap().unwrap(), true);

        // Test i64
        kv.set("i64_key", &42i64).await.unwrap();
        assert_eq!(kv.get::<i64>("i64_key").await.unwrap().unwrap(), 42);

        // Test object
        let person = Person {
            name: "nathan".to_string(),
            age:  30,
        };
        kv.set("person_key", &person).await.unwrap();
        assert_eq!(
            kv.get::<Person>("person_key").await.unwrap().unwrap(),
            person
        );

        // Test remove
        kv.remove("str_key").await.unwrap();
        assert_eq!(kv.get::<String>("str_key").await.unwrap(), None);
    }

    #[tokio::test]
    async fn kv_store_overwrite_test() {
        let tempdir = TempDir::new().unwrap();
        let db_path = tempdir.path().join("test.db");

        let config = DatabaseConfig {
            db_path,
            ..Default::default()
        };
        let db = DBStore::new(config).await.unwrap();
        let kv = db.kv_store();

        // Run migrations
        sqlx::migrate!("./migrations").run(db.pool()).await.unwrap();

        kv.set("key", &"first").await.unwrap();
        assert_eq!(kv.get::<String>("key").await.unwrap().unwrap(), "first");

        kv.set("key", &"second").await.unwrap();
        assert_eq!(kv.get::<String>("key").await.unwrap().unwrap(), "second");
    }

    #[tokio::test]
    async fn kv_store_batch_set_test() {
        let tempdir = TempDir::new().unwrap();
        let db_path = tempdir.path().join("test.db");

        let config = DatabaseConfig {
            db_path,
            ..Default::default()
        };
        let db = DBStore::new(config).await.unwrap();
        let kv = db.kv_store();

        // Run migrations
        sqlx::migrate!("./migrations").run(db.pool()).await.unwrap();

        // Batch set multiple key-value pairs
        let pairs = vec![
            ("batch_key1".to_string(), "value1".to_string()),
            ("batch_key2".to_string(), "value2".to_string()),
            ("batch_key3".to_string(), "value3".to_string()),
        ];
        kv.batch_set(pairs).await.unwrap();

        // Verify all values were set
        assert_eq!(
            kv.get::<String>("batch_key1").await.unwrap().unwrap(),
            "value1"
        );
        assert_eq!(
            kv.get::<String>("batch_key2").await.unwrap().unwrap(),
            "value2"
        );
        assert_eq!(
            kv.get::<String>("batch_key3").await.unwrap().unwrap(),
            "value3"
        );
    }

    #[tokio::test]
    async fn kv_store_batch_get_test() {
        let tempdir = TempDir::new().unwrap();
        let db_path = tempdir.path().join("test.db");

        let config = DatabaseConfig {
            db_path,
            ..Default::default()
        };
        let db = DBStore::new(config).await.unwrap();
        let kv = db.kv_store();

        // Run migrations
        sqlx::migrate!("./migrations").run(db.pool()).await.unwrap();

        // Set up test data
        kv.set("get_key1", &"value1").await.unwrap();
        kv.set("get_key2", &"value2").await.unwrap();
        kv.set("get_key3", &"value3").await.unwrap();

        // Batch get values
        let keys = vec![
            "get_key1".to_string(),
            "get_key2".to_string(),
            "get_key3".to_string(),
            "nonexistent".to_string(),
        ];
        let results = kv.batch_get::<String, _>(keys).await.unwrap();

        // Verify results
        assert_eq!(results.len(), 3); // Only 3 keys exist
        assert_eq!(results.get("get_key1").unwrap(), "value1");
        assert_eq!(results.get("get_key2").unwrap(), "value2");
        assert_eq!(results.get("get_key3").unwrap(), "value3");
        assert_eq!(results.get("nonexistent"), None);
    }

    #[tokio::test]
    async fn kv_store_batch_get_ordered_test() {
        let tempdir = TempDir::new().unwrap();
        let db_path = tempdir.path().join("test.db");

        let config = DatabaseConfig {
            db_path,
            ..Default::default()
        };
        let db = DBStore::new(config).await.unwrap();
        let kv = db.kv_store();

        // Run migrations
        sqlx::migrate!("./migrations").run(db.pool()).await.unwrap();

        // Set up test data
        kv.set("ordered_key1", &"value1").await.unwrap();
        kv.set("ordered_key2", &"value2").await.unwrap();
        kv.set("ordered_key3", &"value3").await.unwrap();

        // Batch get values in order
        let keys = vec![
            "ordered_key1".to_string(),
            "nonexistent".to_string(),
            "ordered_key2".to_string(),
            "ordered_key3".to_string(),
        ];
        let results = kv.batch_get_ordered::<String, _>(keys).await.unwrap();

        // Verify results maintain order
        assert_eq!(results.len(), 4);
        assert_eq!(results[0], Some("value1".to_string()));
        assert_eq!(results[1], None); // nonexistent key
        assert_eq!(results[2], Some("value2".to_string()));
        assert_eq!(results[3], Some("value3".to_string()));
    }

    #[tokio::test]
    async fn kv_store_batch_operations_with_objects_test() {
        let tempdir = TempDir::new().unwrap();
        let db_path = tempdir.path().join("test.db");

        let config = DatabaseConfig {
            db_path,
            ..Default::default()
        };
        let db = DBStore::new(config).await.unwrap();
        let kv = db.kv_store();

        // Run migrations
        sqlx::migrate!("./migrations").run(db.pool()).await.unwrap();

        // Batch set complex objects
        let people = vec![
            (
                "person1".to_string(),
                Person {
                    name: "Alice".to_string(),
                    age:  25,
                },
            ),
            (
                "person2".to_string(),
                Person {
                    name: "Bob".to_string(),
                    age:  30,
                },
            ),
            (
                "person3".to_string(),
                Person {
                    name: "Charlie".to_string(),
                    age:  35,
                },
            ),
        ];
        kv.batch_set(people).await.unwrap();

        // Batch get complex objects
        let keys = vec![
            "person1".to_string(),
            "person2".to_string(),
            "person3".to_string(),
        ];
        let results = kv.batch_get::<Person, _>(keys).await.unwrap();

        assert_eq!(results.len(), 3);
        assert_eq!(results.get("person1").unwrap().name, "Alice");
        assert_eq!(results.get("person2").unwrap().name, "Bob");
        assert_eq!(results.get("person3").unwrap().name, "Charlie");
    }

    #[tokio::test]
    async fn kv_store_get_or_init_key_test() {
        use super::{IdType, KVStoreExt};

        let tempdir = TempDir::new().unwrap();
        let db_path = tempdir.path().join("test.db");

        let config = DatabaseConfig {
            db_path,
            ..Default::default()
        };
        let db = DBStore::new(config).await.unwrap();
        let kv = db.kv_store();

        // Run migrations
        sqlx::migrate!("./migrations").run(db.pool()).await.unwrap();

        // First call should create a new ID
        let result1 = kv.get_or_init_key("test_id").await.unwrap();
        assert!(matches!(result1, IdType::New(_)));
        let id1 = match result1 {
            IdType::New(id) => id,
            _ => panic!("Expected New"),
        };

        // Second call should return existing ID
        let result2 = kv.get_or_init_key("test_id").await.unwrap();
        assert!(matches!(result2, IdType::Existing { .. }));
        let id2 = match result2 {
            IdType::Existing { new_value, .. } => new_value,
            _ => panic!("Expected Existing"),
        };

        assert_eq!(id1, id2);
    }

    #[tokio::test]
    async fn kv_store_batch_get_or_init_keys_test() {
        use super::{IdType, KVStoreExt, KeyRequest};

        let tempdir = TempDir::new().unwrap();
        let db_path = tempdir.path().join("test.db");

        let config = DatabaseConfig {
            db_path,
            ..Default::default()
        };
        let db = DBStore::new(config).await.unwrap();
        let kv = db.kv_store();

        // Run migrations
        sqlx::migrate!("./migrations").run(db.pool()).await.unwrap();

        // Pre-set one key
        kv.set("existing_key", &"existing_id").await.unwrap();

        // Batch get or init keys (mix of existing and new)
        let keys = vec![
            KeyRequest::builder().key("existing_key").build(),
            KeyRequest::builder().key("new_key1").build(),
            KeyRequest::builder().key("new_key2").build(),
        ];
        let results = kv.batch_get_or_init_keys(keys).await.unwrap();

        assert_eq!(results.len(), 3);

        // Existing key should be Existing
        match results.get("existing_key").unwrap() {
            IdType::Existing { previous_value, .. } => assert_eq!(previous_value, "existing_id"),
            _ => panic!("Expected Existing"),
        }

        // New keys should be New
        assert!(matches!(results.get("new_key1").unwrap(), IdType::New(_)));
        assert!(matches!(results.get("new_key2").unwrap(), IdType::New(_)));

        // Verify new keys were actually stored
        let result = kv.get_or_init_key("new_key1").await.unwrap();
        assert!(matches!(result, IdType::Existing { .. }));
    }

    #[tokio::test]
    async fn kv_store_batch_get_or_init_keys_force_test() {
        use super::{IdType, KVStoreExt, KeyRequest};

        let tempdir = TempDir::new().unwrap();
        let db_path = tempdir.path().join("test.db");

        let config = DatabaseConfig {
            db_path,
            ..Default::default()
        };
        let db = DBStore::new(config).await.unwrap();
        let kv = db.kv_store();

        // Run migrations
        sqlx::migrate!("./migrations").run(db.pool()).await.unwrap();

        // Pre-set one key
        kv.set("existing_key", &"existing_id").await.unwrap();

        // Batch get or init keys with force=true for existing key
        let keys = vec![
            KeyRequest::builder()
                .key("existing_key")
                .force(true)
                .build(),
            KeyRequest::builder().key("new_key").build(),
        ];
        let results = kv.batch_get_or_init_keys(keys).await.unwrap();

        assert_eq!(results.len(), 2);

        // Existing key with force=true should have different previous and new values
        match results.get("existing_key").unwrap() {
            IdType::Existing {
                previous_value,
                new_value,
            } => {
                assert_eq!(previous_value, "existing_id");
                assert_ne!(previous_value, new_value); // new_value should be a new UUID
            }
            _ => panic!("Expected Existing"),
        }

        // New key should be New
        assert!(matches!(results.get("new_key").unwrap(), IdType::New(_)));

        // Verify the force-updated key has the new value stored
        let stored_value = kv.get::<String>("existing_key").await.unwrap().unwrap();
        match results.get("existing_key").unwrap() {
            IdType::Existing { new_value, .. } => assert_eq!(&stored_value, new_value),
            _ => panic!("Expected Existing"),
        }
    }
}
