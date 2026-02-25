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

//! Repository trait for application persistence.

use rara_domain_shared::id::ApplicationId;

use super::{
    error::ApplicationError,
    types::{Application, ApplicationFilter, StatusChangeRecord},
};

/// Persistence contract for application aggregates and their status
/// history.
#[async_trait::async_trait]
pub trait ApplicationRepository: Send + Sync {
    /// Persist a new application.
    async fn save(&self, app: &Application) -> Result<Application, ApplicationError>;

    /// Retrieve a single application by its primary key.
    async fn find_by_id(&self, id: ApplicationId) -> Result<Option<Application>, ApplicationError>;

    /// List applications matching the given filter criteria.
    async fn find_all(
        &self,
        filter: &ApplicationFilter,
    ) -> Result<Vec<Application>, ApplicationError>;

    /// Apply an update to an existing application.
    async fn update(&self, app: &Application) -> Result<Application, ApplicationError>;

    /// Soft-delete an application.
    async fn delete(&self, id: ApplicationId) -> Result<(), ApplicationError>;

    /// Persist a status change record.
    async fn save_status_change(&self, record: &StatusChangeRecord)
    -> Result<(), ApplicationError>;

    /// Retrieve the full status history for an application.
    async fn get_status_history(
        &self,
        application_id: ApplicationId,
    ) -> Result<Vec<StatusChangeRecord>, ApplicationError>;
}
