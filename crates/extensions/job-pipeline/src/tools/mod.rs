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

//! Pipeline-specific agent tools.

pub mod job_pipeline_tool;
pub mod list_discovered_jobs_for_scoring;
pub mod pipeline_notify;
pub mod pipeline_tools;
pub mod report_stats;
pub mod run_pipeline_tool;
pub mod save_discovered_job;
pub mod search_jobs_with_job_service;
pub mod update_discovered_job_score_action;

pub use job_pipeline_tool::JobPipelineTool;
pub use list_discovered_jobs_for_scoring::ListDiscoveredJobsForScoringTool;
pub use pipeline_notify::PipelineNotifyTool;
pub use report_stats::ReportPipelineStatsTool;
pub use run_pipeline_tool::{
    CancelJobPipelineTool, PipelineStatusTool, RunJobPipelineTool, UpdateJobPreferencesTool,
};
pub use save_discovered_job::SaveDiscoveredJobTool;
pub use search_jobs_with_job_service::SearchJobsWithJobServiceTool;
pub use update_discovered_job_score_action::UpdateDiscoveredJobScoreActionTool;
