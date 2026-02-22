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

//! AI service — a factory for creating task-specific agents.
//!
//! [`AiService`] holds a [`SettingsSvc`] and creates an LLM client
//! on-demand from runtime settings. Supports OpenRouter and Ollama as
//! providers. If no provider is properly configured, agent factory methods
//! return [`AiError::NotConfigured`].

use rara_domain_shared::settings::{SettingsSvc, model::ModelScenario};
use rig::{
    client::Nothing,
    providers::{ollama, openrouter},
};

use crate::{
    agents::{
        cover_letter::CoverLetterAgent, follow_up::FollowUpDraftAgent,
        interview_prep::InterviewPrepAgent, jd_analyzer::JdAnalyzerAgent, jd_parser::JdParserAgent,
        job_fit::JobFitAgent, resume_analyzer::ResumeAnalyzerAgent,
        resume_optimizer::ResumeOptimizerAgent,
    },
    client::LlmClient,
    error::AiError,
};

/// The AI service — a factory for creating task-specific agents.
///
/// Reads the LLM provider, API key, and model from [`SettingsSvc`] on every
/// call, so configuration changes take effect immediately without restart.
///
/// Supports `"openrouter"` (default) and `"ollama"` as providers.
#[derive(Clone)]
pub struct AiService {
    settings: SettingsSvc,
}

impl AiService {
    /// Create a new `AiService` backed by the given settings service.
    pub fn new(settings: SettingsSvc) -> Self { Self { settings } }

    /// Build an LLM client + model from current settings.
    ///
    /// Returns `Err(AiError::NotConfigured)` when the active provider is
    /// not properly configured (e.g. missing API key for OpenRouter).
    fn client(&self, scenario: ModelScenario) -> Result<(LlmClient, String), AiError> {
        let current = self.settings.current();
        let provider = current.ai.provider.as_deref().unwrap_or("openrouter");
        let model = current.ai.model_for(scenario).to_owned();

        let client = match provider {
            "ollama" => {
                let base_url = current
                    .ai
                    .ollama_base_url
                    .as_deref()
                    .unwrap_or("http://localhost:11434");
                let client = ollama::Client::builder()
                    .api_key(Nothing)
                    .base_url(base_url)
                    .build()
                    .expect("failed to build Ollama client");
                LlmClient::Ollama(client)
            }
            _ => {
                // Default: OpenRouter
                let api_key = current
                    .ai
                    .openrouter_api_key
                    .as_deref()
                    .ok_or(AiError::NotConfigured)?;
                let client = openrouter::Client::builder()
                    .api_key(api_key)
                    .build()
                    .expect("failed to build OpenRouter client");
                LlmClient::OpenRouter(client)
            }
        };

        Ok((client, model))
    }

    fn current_soul_prompt(&self) -> Option<String> {
        let settings_soul = self.settings.current().agent.soul;
        if settings_soul
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
        {
            return settings_soul;
        }
        let markdown_soul = rara_paths::load_agent_soul_prompt();
        if markdown_soul.trim().is_empty() {
            return None;
        }
        Some(markdown_soul)
    }

    /// Create a job-fit analysis agent.
    pub fn job_fit(&self) -> Result<JobFitAgent, AiError> {
        let (client, model) = self.client(ModelScenario::Job)?;
        Ok(JobFitAgent::new(client, model, self.current_soul_prompt()))
    }

    /// Create a resume optimization agent.
    pub fn resume_optimizer(&self) -> Result<ResumeOptimizerAgent, AiError> {
        let (client, model) = self.client(ModelScenario::Job)?;
        Ok(ResumeOptimizerAgent::new(
            client,
            model,
            self.current_soul_prompt(),
        ))
    }

    /// Create an interview preparation agent.
    pub fn interview_prep(&self) -> Result<InterviewPrepAgent, AiError> {
        let (client, model) = self.client(ModelScenario::Job)?;
        Ok(InterviewPrepAgent::new(
            client,
            model,
            self.current_soul_prompt(),
        ))
    }

    /// Create a follow-up email drafting agent.
    pub fn follow_up(&self) -> Result<FollowUpDraftAgent, AiError> {
        let (client, model) = self.client(ModelScenario::Job)?;
        Ok(FollowUpDraftAgent::new(
            client,
            model,
            self.current_soul_prompt(),
        ))
    }

    /// Create a cover letter generation agent.
    pub fn cover_letter(&self) -> Result<CoverLetterAgent, AiError> {
        let (client, model) = self.client(ModelScenario::Job)?;
        Ok(CoverLetterAgent::new(
            client,
            model,
            self.current_soul_prompt(),
        ))
    }

    /// Create a job description parser agent.
    pub fn jd_parser(&self) -> Result<JdParserAgent, AiError> {
        let (client, model) = self.client(ModelScenario::Job)?;
        Ok(JdParserAgent::new(
            client,
            model,
            self.current_soul_prompt(),
        ))
    }

    /// Create a job description analyzer agent.
    pub fn jd_analyzer(&self) -> Result<JdAnalyzerAgent, AiError> {
        let (client, model) = self.client(ModelScenario::Job)?;
        Ok(JdAnalyzerAgent::new(
            client,
            model,
            self.current_soul_prompt(),
        ))
    }

    /// Create a resume analyzer agent.
    pub fn resume_analyzer(&self) -> Result<ResumeAnalyzerAgent, AiError> {
        let (client, model) = self.client(ModelScenario::Job)?;
        Ok(ResumeAnalyzerAgent::new(
            client,
            model,
            self.current_soul_prompt(),
        ))
    }
}
