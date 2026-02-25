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

//! Repository traits for job persistence (discovery).

use async_trait::async_trait;

use super::{error::SourceError, types::NormalizedJob};

// ===========================================================================
// Discovery repository
// ===========================================================================

/// Persistence abstraction for job records.
#[async_trait]
pub trait JobRepository: Send + Sync {
    /// Save a normalized job to the store.
    ///
    /// Returns the persisted job (with DB-generated fields populated).
    async fn save(&self, job: &NormalizedJob) -> Result<NormalizedJob, SourceError>;
}
