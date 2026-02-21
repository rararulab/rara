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

//! HTTP routes for runtime settings.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use serde::Deserialize;
use tokio::fs;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::settings::{
    model::{Settings, UpdateRequest},
    service::SettingsSvc,
};

#[derive(Debug, Clone, Copy)]
struct PromptSpec {
    name:            &'static str,
    description:     &'static str,
    default_content: &'static str,
}

const PROMPT_SPECS: &[PromptSpec] = &[
    PromptSpec {
        name:            "agent/soul.md",
        description:     "Global personality / soul prompt",
        default_content: rara_paths::default_agent_soul_prompt(),
    },
    PromptSpec {
        name:            "chat/default_system.md",
        description:     "Default chat system prompt",
        default_content: include_str!("../../../../../prompts/chat/default_system.md"),
    },
    PromptSpec {
        name:            "workers/agent_policy.md",
        description:     "Proactive/scheduled agent operating policy",
        default_content: include_str!("../../../../../prompts/workers/agent_policy.md"),
    },
    PromptSpec {
        name:            "workers/resume_analysis_instructions.md",
        description:     "Resume analysis tool instructions",
        default_content: include_str!(
            "../../../../../prompts/workers/resume_analysis_instructions.md"
        ),
    },
    PromptSpec {
        name:            "ai/cover_letter.system.md",
        description:     "Cover letter agent system prompt",
        default_content: include_str!("../../../../../prompts/ai/cover_letter.system.md"),
    },
    PromptSpec {
        name:            "ai/follow_up.system.md",
        description:     "Follow-up email agent system prompt",
        default_content: include_str!("../../../../../prompts/ai/follow_up.system.md"),
    },
    PromptSpec {
        name:            "ai/interview_prep.system.md",
        description:     "Interview prep agent system prompt",
        default_content: include_str!("../../../../../prompts/ai/interview_prep.system.md"),
    },
    PromptSpec {
        name:            "ai/jd_analyzer.system.md",
        description:     "Job description analyzer system prompt",
        default_content: include_str!("../../../../../prompts/ai/jd_analyzer.system.md"),
    },
    PromptSpec {
        name:            "ai/jd_parser.system.md",
        description:     "Job description parser system prompt",
        default_content: include_str!("../../../../../prompts/ai/jd_parser.system.md"),
    },
    PromptSpec {
        name:            "ai/job_fit.system.md",
        description:     "Job fit agent system prompt",
        default_content: include_str!("../../../../../prompts/ai/job_fit.system.md"),
    },
    PromptSpec {
        name:            "ai/resume_analyzer.system.md",
        description:     "Resume analyzer system prompt",
        default_content: include_str!("../../../../../prompts/ai/resume_analyzer.system.md"),
    },
    PromptSpec {
        name:            "ai/resume_optimizer.system.md",
        description:     "Resume optimizer system prompt",
        default_content: include_str!("../../../../../prompts/ai/resume_optimizer.system.md"),
    },
];

fn prompt_spec(name: &str) -> Option<&'static PromptSpec> {
    PROMPT_SPECS.iter().find(|spec| spec.name == name)
}

/// Build `/api/v1/settings` routes.
pub fn routes(svc: SettingsSvc) -> OpenApiRouter {
    OpenApiRouter::new()
        .nest(
            "/api/v1",
            OpenApiRouter::new()
                .routes(routes!(get_settings, update_settings))
                .routes(routes!(list_prompts))
                .route(
                    "/settings/prompts/{*name}",
                    get(get_prompt_content).put(update_prompt_content),
                ),
        )
        .with_state(svc)
}

#[utoipa::path(
    get,
    path = "/settings",
    tag = "settings",
    responses(
        (status = 200, description = "Current runtime settings", body = RuntimeSettingsView),
    )
)]
async fn get_settings(
    State(state): State<SettingsSvc>,
) -> Result<Json<RuntimeSettingsView>, (StatusCode, String)> {
    let current = state.current();
    Ok(Json(current.into()))
}

#[utoipa::path(
    post,
    path = "/settings",
    tag = "settings",
    request_body = UpdateRequest,
    responses(
        (status = 200, description = "Settings updated", body = RuntimeSettingsView),
    )
)]
async fn update_settings(
    State(state): State<SettingsSvc>,
    Json(req): Json<UpdateRequest>,
) -> Result<Json<RuntimeSettingsView>, (StatusCode, String)> {
    let updated = state.update(req).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to update runtime settings: {e}"),
        )
    })?;

    Ok(Json(updated.into()))
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct PromptFileView {
    pub name:        String,
    pub description: String,
    pub content:     String,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct PromptListView {
    pub prompts: Vec<PromptFileView>,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct PromptUpdateRequest {
    pub content: String,
}

#[utoipa::path(
    get,
    path = "/settings/prompts",
    tag = "settings",
    responses(
        (status = 200, description = "List of prompt files", body = PromptListView),
    )
)]
async fn list_prompts() -> Result<Json<PromptListView>, (StatusCode, String)> {
    let prompts = PROMPT_SPECS
        .iter()
        .map(|spec| PromptFileView {
            name:        spec.name.to_owned(),
            description: spec.description.to_owned(),
            content:     rara_paths::load_prompt_markdown(spec.name, spec.default_content),
        })
        .collect();

    Ok(Json(PromptListView { prompts }))
}

#[utoipa::path(
    get,
    path = "/settings/prompts/{name}",
    tag = "settings",
    params(("name" = String, Path, description = "Prompt file name")),
    responses(
        (status = 200, description = "Prompt content", body = PromptFileView),
    )
)]
async fn get_prompt_content(
    Path(name): Path<String>,
) -> Result<Json<PromptFileView>, (StatusCode, String)> {
    let name = name.trim_start_matches('/');
    let spec = prompt_spec(name)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("prompt not found: {name}")))?;

    Ok(Json(PromptFileView {
        name:        spec.name.to_owned(),
        description: spec.description.to_owned(),
        content:     rara_paths::load_prompt_markdown(spec.name, spec.default_content),
    }))
}

#[utoipa::path(
    put,
    path = "/settings/prompts/{name}",
    tag = "settings",
    params(("name" = String, Path, description = "Prompt file name")),
    request_body = PromptUpdateRequest,
    responses(
        (status = 200, description = "Prompt updated", body = PromptFileView),
    )
)]
async fn update_prompt_content(
    Path(name): Path<String>,
    Json(req): Json<PromptUpdateRequest>,
) -> Result<Json<PromptFileView>, (StatusCode, String)> {
    let name = name.trim_start_matches('/');
    let spec = prompt_spec(name)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("prompt not found: {name}")))?;

    let path = rara_paths::prompt_markdown_file(spec.name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to create prompt directory: {e}"),
            )
        })?;
    }

    let content = if req.content.trim().is_empty() {
        spec.default_content.to_owned()
    } else {
        req.content
    };

    fs::write(path, &content).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to write prompt markdown: {e}"),
        )
    })?;

    Ok(Json(PromptFileView {
        name: spec.name.to_owned(),
        description: spec.description.to_owned(),
        content,
    }))
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct RuntimeSettingsView {
    pub ai:           AiSettingsView,
    pub telegram:     TgSettingsResp,
    pub agent:        AgentSettingsView,
    pub job_pipeline: JobPipelineSettingsView,
    pub gmail:        GmailSettingsView,
    // TODO: use jiff
    #[schema(value_type = Option<String>)]
    pub updated_at:   Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct AgentSettingsView {
    pub soul:               Option<String>,
    pub chat_system_prompt: Option<String>,
    pub memory:             MemorySettingsView,
    pub composio:           ComposioSettingsView,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct MemorySettingsView {
    pub chroma_url:          Option<String>,
    pub chroma_collection:   Option<String>,
    pub chroma_api_key_hint: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ComposioSettingsView {
    pub api_key:   Option<String>,
    pub entity_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct AiSettingsView {
    pub configured:           bool,
    pub default_model:        Option<String>,
    pub job_model:            Option<String>,
    pub chat_model:           Option<String>,
    pub openrouter_api_key:   Option<String>,
    pub favorite_models:      Vec<String>,
    pub chat_model_fallbacks: Vec<String>,
    pub job_model_fallbacks:  Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct TgSettingsResp {
    pub configured:            bool,
    pub chat_id:               Option<i64>,
    pub allowed_group_chat_id: Option<i64>,
    pub token_hint:            Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct JobPipelineSettingsView {
    pub job_preferences:        Option<String>,
    pub score_threshold_auto:   u8,
    pub score_threshold_notify: u8,
    pub resume_project_path:    Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct GmailSettingsView {
    pub configured:          bool,
    pub auto_send_enabled:   bool,
    pub address:             Option<String>,
    pub app_password_hint:   Option<String>,
}

impl Into<RuntimeSettingsView> for Settings {
    fn into(self) -> RuntimeSettingsView {
        fn secret_hint(secret: Option<&str>) -> Option<String> {
            let secret = secret?;
            let chars: Vec<char> = secret.chars().collect();
            if chars.is_empty() {
                return None;
            }
            if chars.len() <= 4 {
                return Some("*".repeat(chars.len()));
            }
            let suffix: String = chars[chars.len() - 4..].iter().collect();
            Some(format!("***{suffix}"))
        }

        RuntimeSettingsView {
            ai:         AiSettingsView {
                configured:           self.ai.openrouter_api_key.is_some(),
                default_model:        self.ai.default_model.clone(),
                job_model:            self.ai.job_model.clone(),
                chat_model:           self.ai.chat_model.clone(),
                openrouter_api_key:   self.ai.openrouter_api_key.clone(),
                favorite_models:      self.ai.favorite_models.clone(),
                chat_model_fallbacks: self.ai.chat_model_fallbacks.clone(),
                job_model_fallbacks:  self.ai.job_model_fallbacks.clone(),
            },
            telegram:   TgSettingsResp {
                configured:            self.telegram.bot_token.is_some()
                    && self.telegram.chat_id.is_some(),
                chat_id:               self.telegram.chat_id,
                allowed_group_chat_id: self.telegram.allowed_group_chat_id,
                token_hint:            secret_hint(self.telegram.bot_token.as_deref()),
            },
            agent:        AgentSettingsView {
                soul:               self.agent.soul.clone(),
                chat_system_prompt: self.agent.chat_system_prompt.clone(),
                memory:             MemorySettingsView {
                    chroma_url:          self.agent.memory.chroma_url.clone(),
                    chroma_collection:   self.agent.memory.chroma_collection.clone(),
                    chroma_api_key_hint: secret_hint(self.agent.memory.chroma_api_key.as_deref()),
                },
                composio:           ComposioSettingsView {
                    api_key:   self.agent.composio.api_key.clone(),
                    entity_id: self.agent.composio.entity_id.clone(),
                },
            },
            job_pipeline: JobPipelineSettingsView {
                job_preferences:        self.job_pipeline.job_preferences.clone(),
                score_threshold_auto:   self.job_pipeline.score_threshold_auto,
                score_threshold_notify: self.job_pipeline.score_threshold_notify,
                resume_project_path:    self.job_pipeline.resume_project_path.clone(),
            },
            gmail:        GmailSettingsView {
                configured:        self.agent.gmail.address.is_some()
                    && self.agent.gmail.app_password.is_some(),
                auto_send_enabled: self.agent.gmail.auto_send_enabled,
                address:           self.agent.gmail.address.clone(),
                app_password_hint: secret_hint(self.agent.gmail.app_password.as_deref()),
            },
            updated_at:   self.updated_at,
        }
    }
}
