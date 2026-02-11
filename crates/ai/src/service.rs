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

use job_domain_shared::settings::SettingsSvc;
use rig::providers::openrouter;

use crate::{
    agents::{
        cover_letter::CoverLetterAgent, follow_up::FollowUpDraftAgent,
        interview_prep::InterviewPrepAgent, jd_analyzer::JdAnalyzerAgent, jd_parser::JdParserAgent,
        job_fit::JobFitAgent, resume_optimizer::ResumeOptimizerAgent,
    },
    error::AiError,
};

const DEFAULT_MODEL: &str = "openai/gpt-4o";

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
    fn client(&self) -> Result<(openrouter::Client, String), AiError> {
        let current = self.settings.current();
        let api_key = current
            .ai
            .openrouter_api_key
            .as_deref()
            .ok_or(AiError::NotConfigured)?;
        let model = current
            .ai
            .model
            .unwrap_or_else(|| DEFAULT_MODEL.to_owned());
        let client = openrouter::Client::builder()
            .api_key(api_key)
            .build()
            .expect("failed to build OpenRouter client");
        Ok((client, model))
    }

    /// Create a job-fit analysis agent.
    pub fn job_fit(&self) -> Result<JobFitAgent, AiError> {
        let (client, model) = self.client()?;
        Ok(JobFitAgent::new(client, model))
    }

    /// Create a resume optimization agent.
    pub fn resume_optimizer(&self) -> Result<ResumeOptimizerAgent, AiError> {
        let (client, model) = self.client()?;
        Ok(ResumeOptimizerAgent::new(client, model))
    }

    /// Create an interview preparation agent.
    pub fn interview_prep(&self) -> Result<InterviewPrepAgent, AiError> {
        let (client, model) = self.client()?;
        Ok(InterviewPrepAgent::new(client, model))
    }

    /// Create a follow-up email drafting agent.
    pub fn follow_up(&self) -> Result<FollowUpDraftAgent, AiError> {
        let (client, model) = self.client()?;
        Ok(FollowUpDraftAgent::new(client, model))
    }

    /// Create a cover letter generation agent.
    pub fn cover_letter(&self) -> Result<CoverLetterAgent, AiError> {
        let (client, model) = self.client()?;
        Ok(CoverLetterAgent::new(client, model))
    }

    /// Create a job description parser agent.
    pub fn jd_parser(&self) -> Result<JdParserAgent, AiError> {
        let (client, model) = self.client()?;
        Ok(JdParserAgent::new(client, model))
    }

    /// Create a job description analyzer agent.
    pub fn jd_analyzer(&self) -> Result<JdAnalyzerAgent, AiError> {
        let (client, model) = self.client()?;
        Ok(JdAnalyzerAgent::new(client, model))
    }
}
