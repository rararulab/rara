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

pub mod pg_repository;
pub mod repository;
mod router;
pub mod service;
pub mod tools;
pub mod types;

use std::sync::Arc;

use rara_kernel::tool::ToolRegistry;
pub use router::routes;
use service::PipelineService;

use crate::settings::SettingsSvc;

/// Register rara-facing tools (run/cancel/status/preferences) on the main
/// chat agent's tool registry.
///
/// Called by the composition root for the main chat agent.
pub fn register_rara_tools(
    registry: &mut ToolRegistry,
    service: &PipelineService,
    settings_svc: &SettingsSvc,
) {
    registry.register_service(Arc::new(tools::RunJobPipelineTool::new(service.clone())));
    registry.register_service(Arc::new(tools::CancelJobPipelineTool::new(service.clone())));
    registry.register_service(Arc::new(tools::PipelineStatusTool::new(service.clone())));
    registry.register_service(Arc::new(tools::UpdateJobPreferencesTool::new(
        settings_svc.clone(),
    )));
}
