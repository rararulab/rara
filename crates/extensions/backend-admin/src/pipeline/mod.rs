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
