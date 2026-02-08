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

//! Repository traits that define the persistence contract for each domain.
//!
//! Implementations live in the respective domain crates or in an
//! infrastructure adapter crate -- never here.

use crate::id::{ApplicationId, InterviewId, JobSourceId, ResumeId};

/// Repository for job source records.
#[async_trait::async_trait]
pub trait JobSourceRepository: Send + Sync {
    /// Error type returned by this repository.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Retrieve a job source by its unique identifier.
    async fn find_by_id(&self, id: JobSourceId) -> Result<Option<()>, Self::Error>;
}

/// Repository for resume records.
#[async_trait::async_trait]
pub trait ResumeRepository: Send + Sync {
    /// Error type returned by this repository.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Retrieve a resume by its unique identifier.
    async fn find_by_id(&self, id: ResumeId) -> Result<Option<()>, Self::Error>;
}

/// Repository for application records.
#[async_trait::async_trait]
pub trait ApplicationRepository: Send + Sync {
    /// Error type returned by this repository.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Retrieve an application by its unique identifier.
    async fn find_by_id(&self, id: ApplicationId) -> Result<Option<()>, Self::Error>;
}

/// Repository for interview records.
#[async_trait::async_trait]
pub trait InterviewRepository: Send + Sync {
    /// Error type returned by this repository.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Retrieve an interview by its unique identifier.
    async fn find_by_id(&self, id: InterviewId) -> Result<Option<()>, Self::Error>;
}
