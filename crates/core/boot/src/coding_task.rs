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

//! Coding task service factory.

use std::sync::Arc;

/// Create a [`CodingTaskService`](rara_coding_task::service::CodingTaskService).
pub fn init_coding_task_service(
    pool: sqlx::PgPool,
    notify: rara_domain_shared::notify::client::NotifyClient,
    settings: Arc<dyn rara_domain_shared::settings::SettingsProvider>,
    default_repo_url: String,
) -> rara_coding_task::service::CodingTaskService {
    let workspace_manager =
        rara_workspace::WorkspaceManager::new(rara_paths::workspaces_dir().clone());
    rara_coding_task::service::wire(
        pool,
        workspace_manager,
        notify,
        settings,
        default_repo_url,
    )
}
