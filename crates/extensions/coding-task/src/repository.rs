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

//! Repository trait for coding task persistence.

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::CodingTaskError;
use crate::types::{CodingTask, CodingTaskStatus};

#[async_trait]
pub trait CodingTaskRepository: Send + Sync {
    async fn create(&self, task: &CodingTask) -> Result<CodingTask, CodingTaskError>;
    async fn get(&self, id: Uuid) -> Result<CodingTask, CodingTaskError>;
    async fn list(&self) -> Result<Vec<CodingTask>, CodingTaskError>;
    async fn list_by_status(
        &self,
        status: CodingTaskStatus,
    ) -> Result<Vec<CodingTask>, CodingTaskError>;
    async fn update_status(
        &self,
        id: Uuid,
        status: CodingTaskStatus,
    ) -> Result<(), CodingTaskError>;
    async fn update_workspace(
        &self,
        id: Uuid,
        workspace_path: &str,
        tmux_session: &str,
    ) -> Result<(), CodingTaskError>;
    async fn update_pr(
        &self,
        id: Uuid,
        pr_url: &str,
        pr_number: i32,
    ) -> Result<(), CodingTaskError>;
    async fn update_output(
        &self,
        id: Uuid,
        output: &str,
        exit_code: Option<i32>,
    ) -> Result<(), CodingTaskError>;
    async fn update_error(&self, id: Uuid, error: &str) -> Result<(), CodingTaskError>;
    async fn set_started(&self, id: Uuid) -> Result<(), CodingTaskError>;
    async fn set_completed(&self, id: Uuid) -> Result<(), CodingTaskError>;
}
