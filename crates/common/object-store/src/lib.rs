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

//! S3-compatible object storage client for MinIO.
//!
//! Provides a simple put/get/delete/exists interface backed by OpenDAL's S3
//! operator. Designed for use with MinIO but works with any S3-compatible
//! service.

use bytes::Bytes;
use opendal::{Operator, services};
use serde::{Deserialize, Serialize};
use smart_default::SmartDefault;
use snafu::Snafu;
use tracing::debug;

/// Configuration for connecting to an S3-compatible object store.
#[derive(Debug, Clone, Serialize, Deserialize, SmartDefault, bon::Builder)]
pub struct ObjectStoreConfig {
    /// S3 endpoint URL (e.g. `http://localhost:9000` for MinIO).
    #[default = "http://localhost:9000"]
    pub endpoint: String,

    /// Bucket name.
    #[default = "job-data"]
    pub bucket: String,

    /// Access key ID.
    #[default = "minioadmin"]
    pub access_key: String,

    /// Secret access key.
    #[default = "minioadmin"]
    pub secret_key: String,

    /// AWS region (set to any value for MinIO; it is ignored).
    #[default = "us-east-1"]
    pub region: String,

    /// Optional root path prefix for all keys.
    #[default = "/"]
    pub root: String,
}

/// Errors that can occur during object store operations.
#[derive(Debug, Snafu)]
pub enum ObjectStoreError {
    /// Failed to build the S3 operator.
    #[snafu(display("failed to build S3 operator: {source}"))]
    Build { source: opendal::Error },

    /// An S3 put operation failed.
    #[snafu(display("put failed for key '{key}': {source}"))]
    Put {
        key:    String,
        source: opendal::Error,
    },

    /// An S3 get operation failed.
    #[snafu(display("get failed for key '{key}': {source}"))]
    Get {
        key:    String,
        source: opendal::Error,
    },

    /// An S3 delete operation failed.
    #[snafu(display("delete failed for key '{key}': {source}"))]
    Delete {
        key:    String,
        source: opendal::Error,
    },

    /// An S3 stat (exists check) operation failed.
    #[snafu(display("exists check failed for key '{key}': {source}"))]
    Exists {
        key:    String,
        source: opendal::Error,
    },

    /// An S3 list operation failed.
    #[snafu(display("list failed for prefix '{prefix}': {source}"))]
    List {
        prefix: String,
        source: opendal::Error,
    },
}

/// S3-compatible object store backed by MinIO (via OpenDAL).
#[derive(Clone)]
pub struct ObjectStore {
    op: Operator,
}

impl ObjectStore {
    /// Create a new `ObjectStore` from the given configuration.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectStoreError::Build`] if the underlying S3 operator
    /// cannot be constructed (e.g. invalid configuration).
    pub fn new(config: &ObjectStoreConfig) -> Result<Self, ObjectStoreError> {
        let builder = services::S3::default()
            .endpoint(&config.endpoint)
            .bucket(&config.bucket)
            .access_key_id(&config.access_key)
            .secret_access_key(&config.secret_key)
            .region(&config.region)
            .root(&config.root);

        let op = Operator::new(builder)
            .map_err(|source| ObjectStoreError::Build { source })?
            .finish();

        debug!(
            endpoint = %config.endpoint,
            bucket = %config.bucket,
            region = %config.region,
            "object store initialized"
        );

        Ok(Self { op })
    }

    /// Store bytes under the given key.
    ///
    /// Overwrites any existing object at the same key.
    pub async fn put(&self, key: &str, data: Bytes) -> Result<(), ObjectStoreError> {
        self.op
            .write(key, data)
            .await
            .map(|_| ())
            .map_err(|source| ObjectStoreError::Put {
                key: key.to_owned(),
                source,
            })
    }

    /// Retrieve the bytes stored under the given key.
    pub async fn get(&self, key: &str) -> Result<Bytes, ObjectStoreError> {
        let buf = self
            .op
            .read(key)
            .await
            .map_err(|source| ObjectStoreError::Get {
                key: key.to_owned(),
                source,
            })?;
        Ok(buf.to_bytes())
    }

    /// Delete the object at the given key.
    ///
    /// This is idempotent -- deleting a non-existent key is not an error.
    pub async fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
        self.op
            .delete(key)
            .await
            .map_err(|source| ObjectStoreError::Delete {
                key: key.to_owned(),
                source,
            })
    }

    /// Check whether an object exists at the given key.
    pub async fn exists(&self, key: &str) -> Result<bool, ObjectStoreError> {
        self.op
            .exists(key)
            .await
            .map_err(|source| ObjectStoreError::Exists {
                key: key.to_owned(),
                source,
            })
    }

    /// List object keys under the given prefix.
    pub async fn list(&self, prefix: &str) -> Result<Vec<String>, ObjectStoreError> {
        let entries = self
            .op
            .list(prefix)
            .await
            .map_err(|source| ObjectStoreError::List {
                prefix: prefix.to_owned(),
                source,
            })?;
        Ok(entries.into_iter().map(|e| e.path().to_owned()).collect())
    }

    /// Return a reference to the underlying OpenDAL operator for advanced use.
    #[must_use]
    pub fn operator(&self) -> &Operator { &self.op }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        let config = ObjectStoreConfig::default();
        assert_eq!(config.endpoint, "http://localhost:9000");
        assert_eq!(config.bucket, "job-data");
        assert_eq!(config.access_key, "minioadmin");
        assert_eq!(config.secret_key, "minioadmin");
        assert_eq!(config.region, "us-east-1");
        assert_eq!(config.root, "/");
    }

    #[test]
    fn config_builder_works() {
        let config = ObjectStoreConfig::builder()
            .endpoint("http://minio:9000".to_owned())
            .bucket("my-bucket".to_owned())
            .access_key("key".to_owned())
            .secret_key("secret".to_owned())
            .region("eu-west-1".to_owned())
            .root("/data".to_owned())
            .build();
        assert_eq!(config.endpoint, "http://minio:9000");
        assert_eq!(config.bucket, "my-bucket");
        assert_eq!(config.access_key, "key");
        assert_eq!(config.secret_key, "secret");
        assert_eq!(config.region, "eu-west-1");
        assert_eq!(config.root, "/data");
    }

    #[test]
    fn config_serde_roundtrip() {
        let config = ObjectStoreConfig::default();
        let json = serde_json::to_string(&config).expect("serialize");
        let restored: ObjectStoreConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(config.endpoint, restored.endpoint);
        assert_eq!(config.bucket, restored.bucket);
    }

    #[test]
    fn can_construct_store_from_default_config() {
        // This tests that the OpenDAL operator can be built without errors.
        // It does NOT connect to any real S3 endpoint.
        let config = ObjectStoreConfig::default();
        let store = ObjectStore::new(&config);
        assert!(store.is_ok());
    }

    #[test]
    fn error_display() {
        let err = ObjectStoreError::Put {
            key:    "test.txt".to_owned(),
            source: opendal::Error::new(opendal::ErrorKind::Unexpected, "boom"),
        };
        let msg = err.to_string();
        assert!(msg.contains("put failed"));
        assert!(msg.contains("test.txt"));
    }

    /// Integration test that requires a running MinIO instance.
    /// Run with: `cargo test -p job-object-store -- --ignored`
    #[tokio::test]
    #[ignore = "requires a running MinIO instance at localhost:9000"]
    async fn integration_put_get_delete() {
        let config = ObjectStoreConfig::default();
        let store = ObjectStore::new(&config).expect("build store");

        let key = "test/integration.txt";
        let data = Bytes::from_static(b"hello, minio!");

        // put
        store.put(key, data.clone()).await.expect("put");

        // exists
        assert!(store.exists(key).await.expect("exists"));

        // get
        let retrieved = store.get(key).await.expect("get");
        assert_eq!(retrieved, data);

        // delete
        store.delete(key).await.expect("delete");

        // exists after delete
        assert!(!store.exists(key).await.expect("exists after delete"));
    }
}
