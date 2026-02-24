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

use axum::{Json, extract::State, http::StatusCode};
use utoipa_axum::{router::OpenApiRouter, routes};

use rara_domain_shared::settings::{
    model::{
        AgentRuntimeSettingsPatch, AiRuntimeSettingsPatch, ComposioRuntimeSettingsPatch,
        JobPipelineRuntimeSettingsPatch, MemoryRuntimeSettingsPatch, Settings, UpdateRequest,
    },
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
                .merge(rara_domain_shared::settings::ollama::ollama_management_routes()),
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
    request_body = SettingsAdminUpdateRequest,
    responses(
        (status = 200, description = "Settings updated", body = RuntimeSettingsView),
    )
)]
async fn update_settings(
    State(state): State<SettingsSvc>,
    Json(req): Json<SettingsAdminUpdateRequest>,
) -> Result<Json<RuntimeSettingsView>, (StatusCode, String)> {
    let updated = state.update(req.into()).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to update runtime settings: {e}"),
        )
    })?;

    Ok(Json(updated.into()))
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct RuntimeSettingsView {
    pub ai: AiSettingsView,
    pub agent: AgentSettingsView,
    pub job_pipeline: JobPipelineSettingsView,
    pub gmail: GmailSettingsView,
    // TODO: use jiff
    #[schema(value_type = Option<String>)]
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct AgentSettingsView {
    pub memory: MemorySettingsView,
    pub composio: ComposioSettingsView,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct MemorySettingsView {
    pub chroma_url: Option<String>,
    pub chroma_collection: Option<String>,
    pub chroma_api_key_hint: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ComposioSettingsView {
    pub api_key: Option<String>,
    pub entity_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct AiSettingsView {
    pub configured:         bool,
    pub provider:           Option<String>,
    pub ollama_base_url:    Option<String>,
    pub openrouter_api_key: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, utoipa::ToSchema)]
pub struct SettingsAdminUpdateRequest {
    pub ai:           Option<AiSettingsAdminPatch>,
    pub agent:        Option<AgentSettingsAdminPatch>,
    pub job_pipeline: Option<JobPipelineRuntimeSettingsPatch>,
}

impl From<SettingsAdminUpdateRequest> for UpdateRequest {
    fn from(value: SettingsAdminUpdateRequest) -> Self {
        Self {
            ai: value.ai.map(Into::into),
            agent: value.agent.map(Into::into),
            job_pipeline: value.job_pipeline,
            telegram: None,
        }
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize, utoipa::ToSchema)]
pub struct AiSettingsAdminPatch {
    pub openrouter_api_key: Option<String>,
    pub provider:           Option<String>,
    pub ollama_base_url:    Option<String>,
}

impl From<AiSettingsAdminPatch> for AiRuntimeSettingsPatch {
    fn from(value: AiSettingsAdminPatch) -> Self {
        Self {
            openrouter_api_key: value.openrouter_api_key,
            provider: value.provider,
            ollama_base_url: value.ollama_base_url,
            models: None,
            fallback_models: None,
            favorite_models: None,
        }
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize, utoipa::ToSchema)]
pub struct AgentSettingsAdminPatch {
    pub memory:   Option<MemorySettingsAdminPatch>,
    pub composio: Option<ComposioSettingsAdminPatch>,
}

impl From<AgentSettingsAdminPatch> for AgentRuntimeSettingsPatch {
    fn from(value: AgentSettingsAdminPatch) -> Self {
        Self {
            soul: None,
            chat_system_prompt: None,
            proactive_enabled: None,
            proactive_cron: None,
            memory: value.memory.map(Into::into),
            composio: value.composio.map(Into::into),
            gmail: None,
        }
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize, utoipa::ToSchema)]
pub struct MemorySettingsAdminPatch {
    pub chroma_url:        Option<String>,
    pub chroma_collection: Option<String>,
    pub chroma_api_key:    Option<String>,
}

impl From<MemorySettingsAdminPatch> for MemoryRuntimeSettingsPatch {
    fn from(value: MemorySettingsAdminPatch) -> Self {
        Self {
            chroma_url: value.chroma_url,
            chroma_collection: value.chroma_collection,
            chroma_api_key: value.chroma_api_key,
        }
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize, utoipa::ToSchema)]
pub struct ComposioSettingsAdminPatch {
    pub api_key:   Option<String>,
    pub entity_id: Option<String>,
}

impl From<ComposioSettingsAdminPatch> for ComposioRuntimeSettingsPatch {
    fn from(value: ComposioSettingsAdminPatch) -> Self {
        Self {
            api_key: value.api_key,
            entity_id: value.entity_id,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct JobPipelineSettingsView {
    pub job_preferences: Option<String>,
    pub score_threshold_auto: u8,
    pub score_threshold_notify: u8,
    pub resume_project_path: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct GmailSettingsView {
    pub configured: bool,
    pub auto_send_enabled: bool,
    pub address: Option<String>,
    pub app_password_hint: Option<String>,
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
    pub total_ram_gb: f64,
    pub available_ram_gb: f64,
    pub cpu_cores: u32,
    pub cpu_name: String,
    pub has_gpu: bool,
    pub gpu_vram_gb: Option<f64>,
    pub gpu_name: Option<String>,
    pub backend: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct LlmfitModelEntry {
    pub name: String,
    pub provider: Option<serde_json::Value>,
    pub fit_level: String,
    pub run_mode: String,
    pub score: f64,
    pub estimated_tps: f64,
    pub best_quant: String,
    pub memory_required_gb: f64,
    pub use_case: Option<serde_json::Value>,
    pub installed: bool,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct LlmfitRecommendationsResponse {
    pub available: bool,
    pub system: Option<LlmfitSystemInfo>,
    pub models: Vec<LlmfitModelEntry>,
    pub error: Option<String>,
}

#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
pub struct ModelRecommendationsQuery {
    /// Maximum number of models to return (default: 10)
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    10
}

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
                total_ram_gb: specs.total_ram_gb,
                available_ram_gb: specs.available_ram_gb,
                cpu_cores: specs.total_cpu_cores as u32,
                cpu_name: specs.cpu_name.clone(),
                has_gpu: specs.has_gpu,
                gpu_vram_gb: specs.gpu_vram_gb,
                gpu_name: specs.gpu_name.clone(),
                backend: format!("{:?}", specs.backend),
            }),
            models: fits
                .iter()
                .map(|f| LlmfitModelEntry {
                    name: f.model.name.clone(),
                    provider: Some(serde_json::Value::String(f.model.provider.clone())),
                    fit_level: format!("{:?}", f.fit_level),
                    run_mode: format!("{:?}", f.run_mode),
                    score: f.score,
                    estimated_tps: f.estimated_tps,
                    best_quant: f.best_quant.clone(),
                    memory_required_gb: f.memory_required_gb,
                    use_case: Some(serde_json::Value::String(format!("{:?}", f.use_case))),
                    installed: f.installed,
                })
                .collect(),
            error: None,
        }),
        Err(e) => Json(LlmfitRecommendationsResponse {
            available: false,
            system: None,
            models: vec![],
            error: Some(format!("hardware detection failed: {e}")),
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
            ai: AiSettingsView {
                configured: self.ai.is_configured(),
                provider: self.ai.provider.clone(),
                ollama_base_url: self.ai.ollama_base_url.clone(),
                openrouter_api_key: self.ai.openrouter_api_key.clone(),
            },
            agent: AgentSettingsView {
                memory: MemorySettingsView {
                    chroma_url: self.agent.memory.chroma_url.clone(),
                    chroma_collection: self.agent.memory.chroma_collection.clone(),
                    chroma_api_key_hint: secret_hint(self.agent.memory.chroma_api_key.as_deref()),
                },
                composio: ComposioSettingsView {
                    api_key: self.agent.composio.api_key.clone(),
                    entity_id: self.agent.composio.entity_id.clone(),
                },
            },
            job_pipeline: JobPipelineSettingsView {
                job_preferences: self.job_pipeline.job_preferences.clone(),
                score_threshold_auto: self.job_pipeline.score_threshold_auto,
                score_threshold_notify: self.job_pipeline.score_threshold_notify,
                resume_project_path: self.job_pipeline.resume_project_path.clone(),
            },
            gmail: GmailSettingsView {
                configured: self.agent.gmail.address.is_some()
                    && self.agent.gmail.app_password.is_some(),
                auto_send_enabled: self.agent.gmail.auto_send_enabled,
                address: self.agent.gmail.address.clone(),
                app_password_hint: secret_hint(self.agent.gmail.app_password.as_deref()),
            },
            updated_at: self.updated_at,
        }
    }
}
