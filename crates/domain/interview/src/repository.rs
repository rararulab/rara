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

//! Repository trait for interview plan persistence.
//!
//! The trait lives in the domain crate so that the service layer can
//! depend on it without pulling in any infrastructure code.
//! Implementations are expected to live in the infrastructure/store
//! layer.

use async_trait::async_trait;
use job_domain_core::{ApplicationId, InterviewId};

use crate::{
    error::InterviewError,
    types::{InterviewFilter, InterviewPlan},
};

/// Persistence contract for interview plans.
#[async_trait]
pub trait InterviewPlanRepository: Send + Sync {
    /// Persist a new or existing interview plan.
    async fn save(&self, plan: &InterviewPlan) -> Result<InterviewPlan, InterviewError>;

    /// Retrieve a single interview plan by its primary key.
    ///
    /// Returns `None` when the id does not exist or the row is
    /// soft-deleted.
    async fn find_by_id(&self, id: InterviewId) -> Result<Option<InterviewPlan>, InterviewError>;

    /// Retrieve all interview plans for a given application.
    async fn find_by_application(
        &self,
        app_id: ApplicationId,
    ) -> Result<Vec<InterviewPlan>, InterviewError>;

    /// List interview plans matching the given filter criteria.
    async fn find_all(
        &self,
        filter: &InterviewFilter,
    ) -> Result<Vec<InterviewPlan>, InterviewError>;

    /// Apply updates to an existing interview plan.
    async fn update(&self, plan: &InterviewPlan) -> Result<InterviewPlan, InterviewError>;

    /// Soft-delete an interview plan by id.
    async fn delete(&self, id: InterviewId) -> Result<(), InterviewError>;
}
