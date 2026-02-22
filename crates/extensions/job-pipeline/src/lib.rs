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

//! # rara-ext-job-pipeline
//!
//! Job pipeline extension -- agent-driven job discovery, scoring, and resume
//! optimization.
//!
//! ## Public API
//!
//! - [`service::PipelineService`] -- manages the pipeline agent lifecycle.
//! - [`routes::routes`] -- HTTP routes for pipeline management.
//! - [`tools`] -- agent tools (pipeline-internal and rara-facing).
//! - [`register_rara_tools`] -- register rara-facing pipeline tools.

pub mod pg_repository;
pub mod repository;
pub mod routes;
pub mod service;
pub mod tools;
pub mod types;

use std::sync::Arc;

use rara_agents::tool_registry::ToolRegistry;
use service::PipelineService;

/// Register rara-facing tools (run/cancel/status pipeline) on the main
/// chat agent's tool registry.
///
/// Called by the composition root for the main chat agent.
pub fn register_rara_tools(registry: &mut ToolRegistry, service: &PipelineService) {
    registry.register_service(Arc::new(
        tools::RunJobPipelineTool::new(service.clone()),
    ));
    registry.register_service(Arc::new(
        tools::CancelJobPipelineTool::new(service.clone()),
    ));
    registry.register_service(Arc::new(
        tools::PipelineStatusTool::new(service.clone()),
    ));
}
