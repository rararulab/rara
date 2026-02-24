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

//! Task-specific AI agents.
//!
//! Each agent is a lightweight struct that holds an LLM provider reference
//! and model name, executing single-turn prompt -> completion calls.

pub mod completion;
pub mod cover_letter;
pub mod error;
pub mod follow_up;
pub mod interview_prep;
pub mod jd_analyzer;
pub mod jd_parser;
pub mod job_fit;
pub mod resume_analyzer;
pub mod resume_optimizer;

use std::sync::Arc;

use agent_core::provider::LlmProviderLoaderRef;
use rara_domain_shared::settings::{SettingsSvc, model::ModelScenario};

use crate::builtin::tasks::{
    cover_letter::CoverLetterAgent, error::TaskAgentError, follow_up::FollowUpDraftAgent,
    interview_prep::InterviewPrepAgent, jd_analyzer::JdAnalyzerAgent, jd_parser::JdParserAgent,
    job_fit::JobFitAgent, resume_analyzer::ResumeAnalyzerAgent,
    resume_optimizer::ResumeOptimizerAgent,
};

/// The task agent service -- a factory for creating task-specific agents.
///
/// Reads the model name from [`SettingsSvc`] and acquires an LLM provider
/// on every call, so configuration changes take effect immediately without
/// restart.
#[derive(Clone)]
pub struct TaskAgentService {
    settings:     SettingsSvc,
    llm_provider: LlmProviderLoaderRef,
    prompt_repo:  Arc<dyn agent_core::prompt::PromptRepo>,
}

impl TaskAgentService {
    /// Create a new `TaskAgentService`.
    pub fn new(
        settings: SettingsSvc,
        llm_provider: LlmProviderLoaderRef,
        prompt_repo: Arc<dyn agent_core::prompt::PromptRepo>,
    ) -> Self {
        Self {
            settings,
            llm_provider,
            prompt_repo,
        }
    }

    /// Acquire the current LLM provider and model name for the given scenario.
    async fn provider_and_model(
        &self,
        scenario: ModelScenario,
    ) -> Result<(Arc<dyn agent_core::provider::LlmProvider>, String), TaskAgentError> {
        let current = self.settings.current();
        let model = current.ai.model_for(scenario).to_owned();

        let provider = self
            .llm_provider
            .acquire_provider()
            .await
            .map_err(|_| TaskAgentError::NotConfigured)?;

        Ok((provider, model))
    }

    /// Create a job-fit analysis agent.
    pub async fn job_fit(&self) -> Result<JobFitAgent, TaskAgentError> {
        let (provider, model) = self.provider_and_model(ModelScenario::Job).await?;
        Ok(JobFitAgent::new(provider, model, self.prompt_repo.clone()))
    }

    /// Create a resume optimization agent.
    pub async fn resume_optimizer(&self) -> Result<ResumeOptimizerAgent, TaskAgentError> {
        let (provider, model) = self.provider_and_model(ModelScenario::Job).await?;
        Ok(ResumeOptimizerAgent::new(
            provider,
            model,
            self.prompt_repo.clone(),
        ))
    }

    /// Create an interview preparation agent.
    pub async fn interview_prep(&self) -> Result<InterviewPrepAgent, TaskAgentError> {
        let (provider, model) = self.provider_and_model(ModelScenario::Job).await?;
        Ok(InterviewPrepAgent::new(
            provider,
            model,
            self.prompt_repo.clone(),
        ))
    }

    /// Create a follow-up email drafting agent.
    pub async fn follow_up(&self) -> Result<FollowUpDraftAgent, TaskAgentError> {
        let (provider, model) = self.provider_and_model(ModelScenario::Job).await?;
        Ok(FollowUpDraftAgent::new(
            provider,
            model,
            self.prompt_repo.clone(),
        ))
    }

    /// Create a cover letter generation agent.
    pub async fn cover_letter(&self) -> Result<CoverLetterAgent, TaskAgentError> {
        let (provider, model) = self.provider_and_model(ModelScenario::Job).await?;
        Ok(CoverLetterAgent::new(
            provider,
            model,
            self.prompt_repo.clone(),
        ))
    }

    /// Create a job description parser agent.
    pub async fn jd_parser(&self) -> Result<JdParserAgent, TaskAgentError> {
        let (provider, model) = self.provider_and_model(ModelScenario::Job).await?;
        Ok(JdParserAgent::new(
            provider,
            model,
            self.prompt_repo.clone(),
        ))
    }

    /// Create a job description analyzer agent.
    pub async fn jd_analyzer(&self) -> Result<JdAnalyzerAgent, TaskAgentError> {
        let (provider, model) = self.provider_and_model(ModelScenario::Job).await?;
        Ok(JdAnalyzerAgent::new(
            provider,
            model,
            self.prompt_repo.clone(),
        ))
    }

    /// Create a resume analyzer agent.
    pub async fn resume_analyzer(&self) -> Result<ResumeAnalyzerAgent, TaskAgentError> {
        let (provider, model) = self.provider_and_model(ModelScenario::Job).await?;
        Ok(ResumeAnalyzerAgent::new(
            provider,
            model,
            self.prompt_repo.clone(),
        ))
    }
}
