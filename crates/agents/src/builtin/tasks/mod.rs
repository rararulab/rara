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
//! and model name. Agents operate in one of two modes:
//!
//! - **SingleRound** (default): single-turn prompt -> completion call.
//! - **WithTools**: multi-round tool-calling loop for analysis agents.
//!
//! The tool-calling mode is opt-in: analysis agents (`JobFitAgent`,
//! `JdParserAgent`, `JdAnalyzerAgent`, `ResumeAnalyzerAgent`) support
//! `.with_tools()` to enable it. Generation agents remain single-round.

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
use agent_core::tool_registry::ToolRegistry;
use rara_domain_shared::settings::SettingsSvc;

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
///
/// An optional [`ToolRegistry`] can be attached via [`with_tools`](Self::with_tools)
/// to enable tool-calling mode for analysis agents.
#[derive(Clone)]
pub struct TaskAgentService {
    settings:     SettingsSvc,
    llm_provider: LlmProviderLoaderRef,
    prompt_repo:  Arc<dyn agent_core::prompt::PromptRepo>,
    /// Optional tool registry for analysis agents. When set, analysis agents
    /// (job_fit, jd_parser, jd_analyzer, resume_analyzer) are created with
    /// tool-calling capability.
    tools:        Option<Arc<ToolRegistry>>,
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
            tools: None,
        }
    }

    /// Attach a tool registry to enable tool-calling mode for analysis agents.
    ///
    /// This is opt-in: only analysis agents (job_fit, jd_parser, jd_analyzer,
    /// resume_analyzer) will use tools. Generation agents (cover_letter,
    /// interview_prep, follow_up, resume_optimizer) remain single-round.
    #[must_use]
    pub fn with_tools(mut self, tools: Arc<ToolRegistry>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Acquire the current LLM provider and model name for the given key.
    async fn provider_and_model(
        &self,
        key: &str,
    ) -> Result<(Arc<dyn agent_core::provider::LlmProvider>, String), TaskAgentError> {
        let current = self.settings.current();
        let model = current.ai.model_for_key(key);

        let provider = self
            .llm_provider
            .acquire_provider()
            .await
            .map_err(|_| TaskAgentError::NotConfigured)?;

        Ok((provider, model))
    }

    /// Create a job-fit analysis agent.
    ///
    /// If a tool registry is attached, the agent is created with tool-calling
    /// capability.
    pub async fn job_fit(&self) -> Result<JobFitAgent, TaskAgentError> {
        let (provider, model) = self.provider_and_model("job").await?;
        let mut agent = JobFitAgent::new(provider, model, self.prompt_repo.clone());
        if let Some(ref tools) = self.tools {
            agent = agent.with_tools(Arc::clone(tools));
        }
        Ok(agent)
    }

    /// Create a resume optimization agent (single-round, no tools).
    pub async fn resume_optimizer(&self) -> Result<ResumeOptimizerAgent, TaskAgentError> {
        let (provider, model) = self.provider_and_model("job").await?;
        Ok(ResumeOptimizerAgent::new(
            provider,
            model,
            self.prompt_repo.clone(),
        ))
    }

    /// Create an interview preparation agent (single-round, no tools).
    pub async fn interview_prep(&self) -> Result<InterviewPrepAgent, TaskAgentError> {
        let (provider, model) = self.provider_and_model("job").await?;
        Ok(InterviewPrepAgent::new(
            provider,
            model,
            self.prompt_repo.clone(),
        ))
    }

    /// Create a follow-up email drafting agent (single-round, no tools).
    pub async fn follow_up(&self) -> Result<FollowUpDraftAgent, TaskAgentError> {
        let (provider, model) = self.provider_and_model("job").await?;
        Ok(FollowUpDraftAgent::new(
            provider,
            model,
            self.prompt_repo.clone(),
        ))
    }

    /// Create a cover letter generation agent (single-round, no tools).
    pub async fn cover_letter(&self) -> Result<CoverLetterAgent, TaskAgentError> {
        let (provider, model) = self.provider_and_model("job").await?;
        Ok(CoverLetterAgent::new(
            provider,
            model,
            self.prompt_repo.clone(),
        ))
    }

    /// Create a job description parser agent.
    ///
    /// If a tool registry is attached, the agent is created with tool-calling
    /// capability.
    pub async fn jd_parser(&self) -> Result<JdParserAgent, TaskAgentError> {
        let (provider, model) = self.provider_and_model("job").await?;
        let mut agent = JdParserAgent::new(provider, model, self.prompt_repo.clone());
        if let Some(ref tools) = self.tools {
            agent = agent.with_tools(Arc::clone(tools));
        }
        Ok(agent)
    }

    /// Create a job description analyzer agent.
    ///
    /// If a tool registry is attached, the agent is created with tool-calling
    /// capability.
    pub async fn jd_analyzer(&self) -> Result<JdAnalyzerAgent, TaskAgentError> {
        let (provider, model) = self.provider_and_model("job").await?;
        let mut agent = JdAnalyzerAgent::new(provider, model, self.prompt_repo.clone());
        if let Some(ref tools) = self.tools {
            agent = agent.with_tools(Arc::clone(tools));
        }
        Ok(agent)
    }

    /// Create a resume analyzer agent.
    ///
    /// If a tool registry is attached, the agent is created with tool-calling
    /// capability.
    pub async fn resume_analyzer(&self) -> Result<ResumeAnalyzerAgent, TaskAgentError> {
        let (provider, model) = self.provider_and_model("job").await?;
        let mut agent = ResumeAnalyzerAgent::new(provider, model, self.prompt_repo.clone());
        if let Some(ref tools) = self.tools {
            agent = agent.with_tools(Arc::clone(tools));
        }
        Ok(agent)
    }
}
