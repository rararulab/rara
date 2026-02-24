use axum::{Json, extract::State, http::StatusCode};
use utoipa_axum::{router::OpenApiRouter, routes};

use rara_domain_shared::settings::{
    model::{AiRuntimeSettingsPatch, Settings, UpdateRequest},
    service::SettingsSvc,
};

pub fn routes(svc: SettingsSvc) -> OpenApiRouter {
    OpenApiRouter::new()
        .nest(
            "/api/v1",
            OpenApiRouter::new()
                .routes(routes!(get_ai_settings, update_ai_settings))
                .routes(routes!(get_ollama_model_recommendations))
                .merge(rara_domain_shared::settings::ollama::ollama_management_routes()),
        )
        .with_state(svc)
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct AiAdminSettingsView {
    pub configured: bool,
    pub provider: Option<String>,
    pub ollama_base_url: Option<String>,
    pub openrouter_api_key: Option<String>,
    #[schema(value_type = Option<String>)]
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, utoipa::ToSchema)]
pub struct AiAdminUpdateRequest {
    pub openrouter_api_key: Option<String>,
    pub provider: Option<String>,
    pub ollama_base_url: Option<String>,
}

#[utoipa::path(
    get,
    path = "/ai/settings",
    tag = "ai-admin",
    responses((status = 200, description = "AI provider runtime settings", body = AiAdminSettingsView))
)]
async fn get_ai_settings(State(state): State<SettingsSvc>) -> Json<AiAdminSettingsView> {
    Json(AiAdminSettingsView::from(state.current()))
}

#[utoipa::path(
    put,
    path = "/ai/settings",
    tag = "ai-admin",
    request_body = AiAdminUpdateRequest,
    responses((status = 200, description = "AI provider settings updated", body = AiAdminSettingsView))
)]
async fn update_ai_settings(
    State(state): State<SettingsSvc>,
    Json(req): Json<AiAdminUpdateRequest>,
) -> Result<Json<AiAdminSettingsView>, (StatusCode, String)> {
    let updated = state
        .update(UpdateRequest {
            ai: Some(AiRuntimeSettingsPatch {
                openrouter_api_key: req.openrouter_api_key,
                provider: req.provider,
                ollama_base_url: req.ollama_base_url,
                models: None,
                fallback_models: None,
                favorite_models: None,
            }),
            ..UpdateRequest::default()
        })
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to update ai settings: {e}"),
            )
        })?;

    Ok(Json(AiAdminSettingsView::from(updated)))
}

impl From<Settings> for AiAdminSettingsView {
    fn from(value: Settings) -> Self {
        Self {
            configured: value.ai.is_configured(),
            provider: value.ai.provider,
            ollama_base_url: value.ai.ollama_base_url,
            openrouter_api_key: value.ai.openrouter_api_key,
            updated_at: value.updated_at,
        }
    }
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
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize { 10 }

#[utoipa::path(
    get,
    path = "/ai/ollama/model-recommendations",
    tag = "ai-admin",
    params(ModelRecommendationsQuery),
    responses((status = 200, description = "llmfit model recommendations", body = LlmfitRecommendationsResponse))
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
