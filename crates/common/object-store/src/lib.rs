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

//! S3-compatible object storage configuration and OpenDAL operator builder.

use opendal::{Operator, services};
use serde::{Deserialize, Serialize};
use smart_default::SmartDefault;
use snafu::{ResultExt, Snafu};
use tracing::{info, instrument};

pub type Result<T> = std::result::Result<T, ObjectStoreError>;

/// Errors that can occur while building the object-store operator.
#[derive(Debug, Snafu)]
pub enum ObjectStoreError {
    /// Failed to build the S3 operator.
    #[snafu(display("failed to operate s3 via opendal: {source}"))]
    Opendal { source: opendal::Error },
}

/// Configuration for connecting to an S3-compatible object store.
#[derive(Debug, Clone, Serialize, Deserialize, SmartDefault, bon::Builder)]
pub struct ObjectStoreConfig {
    /// S3 endpoint URL (e.g. `http://localhost:9000` for MinIO).
    #[default = "http://localhost:9000"]
    pub endpoint: String,

    /// Bucket name.
    #[default = "rara"]
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
}

impl ObjectStoreConfig {
    /// Build an OpenDAL S3 operator from object-store configuration.
    #[instrument]
    pub fn open(&self) -> Result<Operator> {
        let builder = services::S3::default()
            .endpoint(&self.endpoint)
            .bucket(&self.bucket)
            .access_key_id(&self.access_key)
            .secret_access_key(&self.secret_key)
            .region(&self.region)
            .root("/");

        let op = Operator::new(builder).context(OpendalSnafu)?.finish();

        info!(
            endpoint = %self.endpoint,
            bucket = %self.bucket,
            region = %self.region,
            "object store operator initialized"
        );

        Ok(op)
    }
}
