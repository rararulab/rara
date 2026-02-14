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
//! [`AiService`] holds a [`SettingsSvc`] and creates an OpenRouter client
//! on-demand from runtime settings. If the user hasn't configured an API
//! key, agent factory methods return [`AiError::NotConfigured`].

use rara_domain_shared::settings::{SettingsSvc, model::ModelScenario};
use rig::providers::openrouter;

use crate::{
    agents::{
        cover_letter::CoverLetterAgent, follow_up::FollowUpDraftAgent,
        interview_prep::InterviewPrepAgent, jd_analyzer::JdAnalyzerAgent, jd_parser::JdParserAgent,
        job_fit::JobFitAgent, resume_analyzer::ResumeAnalyzerAgent,
        resume_optimizer::ResumeOptimizerAgent,
    },
    error::AiError,
};

/// The AI service — a factory for creating task-specific agents.
///
/// Reads the OpenRouter API key and model from [`SettingsSvc`] on every
/// call, so configuration changes take effect immediately without restart.
#[derive(Clone)]
pub struct AiService {
    settings: SettingsSvc,
}

impl AiService {
    /// Create a new `AiService` backed by the given settings service.
    pub fn new(settings: SettingsSvc) -> Self { Self { settings } }

    /// Build an OpenRouter client + model from current settings.
    ///
    /// Returns `Err(AiError::NotConfigured)` when no API key is set.
    fn client(&self, scenario: ModelScenario) -> Result<(openrouter::Client, String), AiError> {
        let current = self.settings.current();
        let api_key = current
            .ai
            .openrouter_api_key
            .as_deref()
            .ok_or(AiError::NotConfigured)?;
        let model = current.ai.model_for(scenario).to_owned();
        let client = openrouter::Client::builder()
            .api_key(api_key)
            .build()
            .expect("failed to build OpenRouter client");
        Ok((client, model))
    }

    /// Create a job-fit analysis agent.
    pub fn job_fit(&self) -> Result<JobFitAgent, AiError> {
        let (client, model) = self.client(ModelScenario::Job)?;
        Ok(JobFitAgent::new(client, model))
    }

    /// Create a resume optimization agent.
    pub fn resume_optimizer(&self) -> Result<ResumeOptimizerAgent, AiError> {
        let (client, model) = self.client(ModelScenario::Job)?;
        Ok(ResumeOptimizerAgent::new(client, model))
    }

    /// Create an interview preparation agent.
    pub fn interview_prep(&self) -> Result<InterviewPrepAgent, AiError> {
        let (client, model) = self.client(ModelScenario::Job)?;
        Ok(InterviewPrepAgent::new(client, model))
    }

    /// Create a follow-up email drafting agent.
    pub fn follow_up(&self) -> Result<FollowUpDraftAgent, AiError> {
        let (client, model) = self.client(ModelScenario::Job)?;
        Ok(FollowUpDraftAgent::new(client, model))
    }

    /// Create a cover letter generation agent.
    pub fn cover_letter(&self) -> Result<CoverLetterAgent, AiError> {
        let (client, model) = self.client(ModelScenario::Job)?;
        Ok(CoverLetterAgent::new(client, model))
    }

    /// Create a job description parser agent.
    pub fn jd_parser(&self) -> Result<JdParserAgent, AiError> {
        let (client, model) = self.client(ModelScenario::Job)?;
        Ok(JdParserAgent::new(client, model))
    }

    /// Create a job description analyzer agent.
    pub fn jd_analyzer(&self) -> Result<JdAnalyzerAgent, AiError> {
        let (client, model) = self.client(ModelScenario::Job)?;
        Ok(JdAnalyzerAgent::new(client, model))
    }

    /// Create a resume analyzer agent.
    pub fn resume_analyzer(&self) -> Result<ResumeAnalyzerAgent, AiError> {
        let (client, model) = self.client(ModelScenario::Job)?;
        Ok(ResumeAnalyzerAgent::new(client, model))
    }
}
