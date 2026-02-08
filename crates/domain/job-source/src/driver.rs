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

//! Job source driver trait and supporting types.

use job_domain_core::JobSourceId;
use serde::{Deserialize, Serialize};

/// A single job listing fetched from an external source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobListing {
    /// Human-readable title of the position.
    pub title: String,
    /// Company offering the position.
    pub company: String,
    /// URL to the original listing.
    pub url: String,
    /// Optional location (remote / city).
    pub location: Option<String>,
}

/// Trait that every job source driver must implement.
///
/// Drivers are responsible for fetching job listings from a specific platform
/// and converting them into the canonical [`JobListing`] type.
#[async_trait::async_trait]
pub trait JobSourceDriver: Send + Sync {
    /// Human-readable name of this source (e.g. "LinkedIn").
    fn name(&self) -> &str;

    /// Unique identifier for the configured source instance.
    fn source_id(&self) -> JobSourceId;

    /// Fetch the latest batch of job listings from this source.
    async fn fetch_listings(&self) -> Result<Vec<JobListing>, Box<dyn std::error::Error + Send + Sync>>;
}
