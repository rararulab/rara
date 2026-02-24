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
    extract::State,
    http::StatusCode,
};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::settings::{
    model::{Settings, UpdateRequest},
    service::SettingsSvc,
};

/// Build `/api/v1/settings` routes.
pub fn routes(svc: SettingsSvc) -> OpenApiRouter {
    OpenApiRouter::new()
        .nest(
            "/api/v1",
            OpenApiRouter::new()
                .routes(routes!(get_settings, update_settings))
                .routes(routes!(get_ssh_key))
                .routes(routes!(get_ollama_model_recommendations))
                .merge(crate::settings::ollama::ollama_management_routes()),
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
    pub provider:             Option<String>,
    pub ollama_base_url:      Option<String>,
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
    pub configured:              bool,
    pub chat_id:                 Option<i64>,
    pub allowed_group_chat_id:   Option<i64>,
    pub notification_channel_id: Option<i64>,
    pub token_hint:              Option<String>,
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

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct SshKeyResponse {
    pub public_key: String,
}

#[utoipa::path(
    get,
    path = "/settings/ssh-key",
    tag = "settings",
    responses(
        (status = 200, description = "SSH public key", body = SshKeyResponse),
    )
)]
async fn get_ssh_key() -> Result<Json<SshKeyResponse>, (StatusCode, String)> {
    let ssh_dir = rara_paths::data_dir().join("ssh");
    let public_key = rara_git::get_public_key(&ssh_dir)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(SshKeyResponse { public_key }))
}


#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct LlmfitSystemInfo {
    pub total_ram_gb:     f64,
    pub available_ram_gb: f64,
    pub cpu_cores:        u32,
    pub cpu_name:         String,
    pub has_gpu:          bool,
    pub gpu_vram_gb:      Option<f64>,
    pub gpu_name:         Option<String>,
    pub backend:          String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct LlmfitModelEntry {
    pub name:               String,
    pub provider:           Option<serde_json::Value>,
    pub fit_level:          String,
    pub run_mode:           String,
    pub score:              f64,
    pub estimated_tps:      f64,
    pub best_quant:         String,
    pub memory_required_gb: f64,
    pub use_case:           Option<serde_json::Value>,
    pub installed:          bool,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct LlmfitRecommendationsResponse {
    pub available: bool,
    pub system:    Option<LlmfitSystemInfo>,
    pub models:    Vec<LlmfitModelEntry>,
    pub error:     Option<String>,
}

#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
pub struct ModelRecommendationsQuery {
    /// Maximum number of models to return (default: 10)
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize { 10 }

#[utoipa::path(
    get,
    path = "/settings/ollama/model-recommendations",
    tag = "settings",
    params(ModelRecommendationsQuery),
    responses(
        (status = 200, description = "llmfit model recommendations", body = LlmfitRecommendationsResponse),
    )
)]
async fn get_ollama_model_recommendations(
    axum::extract::Query(query): axum::extract::Query<ModelRecommendationsQuery>,
) -> Json<LlmfitRecommendationsResponse> {
    let limit = query.limit;
    let result = tokio::task::spawn_blocking(move || {
        let specs = llmfit_core::hardware::SystemSpecs::detect();
        let db = llmfit_core::models::ModelDatabase::new();
        let mut fits: Vec<llmfit_core::fit::ModelFit> = db
            .get_all_models()
            .iter()
            .map(|m| llmfit_core::fit::ModelFit::analyze(m, &specs))
            .collect();
        fits = llmfit_core::fit::rank_models_by_fit(fits);
        fits.truncate(limit);
        (specs, fits)
    })
    .await;

    match result {
        Ok((specs, fits)) => Json(LlmfitRecommendationsResponse {
            available: true,
            system: Some(LlmfitSystemInfo {
                total_ram_gb:     specs.total_ram_gb,
                available_ram_gb: specs.available_ram_gb,
                cpu_cores:        specs.total_cpu_cores as u32,
                cpu_name:         specs.cpu_name.clone(),
                has_gpu:          specs.has_gpu,
                gpu_vram_gb:      specs.gpu_vram_gb,
                gpu_name:         specs.gpu_name.clone(),
                backend:          format!("{:?}", specs.backend),
            }),
            models: fits
                .iter()
                .map(|f| LlmfitModelEntry {
                    name:               f.model.name.clone(),
                    provider:           Some(serde_json::Value::String(f.model.provider.clone())),
                    fit_level:          format!("{:?}", f.fit_level),
                    run_mode:           format!("{:?}", f.run_mode),
                    score:              f.score,
                    estimated_tps:      f.estimated_tps,
                    best_quant:         f.best_quant.clone(),
                    memory_required_gb: f.memory_required_gb,
                    use_case:           Some(serde_json::Value::String(format!("{:?}", f.use_case))),
                    installed:          f.installed,
                })
                .collect(),
            error: None,
        }),
        Err(e) => Json(LlmfitRecommendationsResponse {
            available: false,
            system:    None,
            models:    vec![],
            error:     Some(format!("hardware detection failed: {e}")),
        }),
    }
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
                configured:           self.ai.is_configured(),
                provider:             self.ai.provider.clone(),
                ollama_base_url:      self.ai.ollama_base_url.clone(),
                default_model:        self.ai.default_model.clone(),
                job_model:            self.ai.job_model.clone(),
                chat_model:           self.ai.chat_model.clone(),
                openrouter_api_key:   self.ai.openrouter_api_key.clone(),
                favorite_models:      self.ai.favorite_models.clone(),
                chat_model_fallbacks: self.ai.chat_model_fallbacks.clone(),
                job_model_fallbacks:  self.ai.job_model_fallbacks.clone(),
            },
            telegram:   TgSettingsResp {
                configured:              self.telegram.bot_token.is_some()
                    && self.telegram.chat_id.is_some(),
                chat_id:                 self.telegram.chat_id,
                allowed_group_chat_id:   self.telegram.allowed_group_chat_id,
                notification_channel_id: self.telegram.notification_channel_id,
                token_hint:              secret_hint(self.telegram.bot_token.as_deref()),
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
