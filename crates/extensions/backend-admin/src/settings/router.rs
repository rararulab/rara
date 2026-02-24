use axum::{Json, extract::State, http::StatusCode};
use utoipa_axum::{router::OpenApiRouter, routes};

use rara_domain_shared::settings::{
    model::{
        AgentRuntimeSettingsPatch, ComposioRuntimeSettingsPatch, JobPipelineRuntimeSettingsPatch,
        Settings, UpdateRequest,
    },
    service::SettingsSvc,
};

pub fn routes(svc: SettingsSvc) -> OpenApiRouter {
    OpenApiRouter::new()
        .merge(runtime_routes())
        .merge(super::ai::routes())
        .merge(super::gmail::routes())
        .merge(super::auth::routes())
        .merge(super::tg::routes())
        .with_state(svc)
}

fn runtime_routes() -> OpenApiRouter<SettingsSvc> {
    OpenApiRouter::new().nest(
        "/api/v1",
        OpenApiRouter::new().routes(routes!(get_settings, update_settings)),
    )
}

#[utoipa::path(
    get,
    path = "/settings",
    tag = "settings",
    responses((status = 200, description = "Runtime settings (unsplit admins only)", body = RuntimeSettingsView))
)]
async fn get_settings(
    State(state): State<SettingsSvc>,
) -> Result<Json<RuntimeSettingsView>, (StatusCode, String)> {
    Ok(Json(state.current().into()))
}

#[utoipa::path(
    post,
    path = "/settings",
    tag = "settings",
    request_body = SettingsAdminUpdateRequest,
    responses((status = 200, description = "Settings updated", body = RuntimeSettingsView))
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
    pub agent: AgentSettingsView,
    pub job_pipeline: JobPipelineSettingsView,
    #[schema(value_type = Option<String>)]
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct AgentSettingsView {
    pub composio: ComposioSettingsView,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ComposioSettingsView {
    pub api_key: Option<String>,
    pub entity_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct JobPipelineSettingsView {
    pub job_preferences: Option<String>,
    pub score_threshold_auto: u8,
    pub score_threshold_notify: u8,
    pub resume_project_path: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, utoipa::ToSchema)]
pub struct SettingsAdminUpdateRequest {
    pub agent: Option<AgentSettingsAdminPatch>,
    pub job_pipeline: Option<JobPipelineRuntimeSettingsPatch>,
}

impl From<SettingsAdminUpdateRequest> for UpdateRequest {
    fn from(value: SettingsAdminUpdateRequest) -> Self {
        let mut req = UpdateRequest::default();
        req.agent = value.agent.map(Into::into);
        req.job_pipeline = value.job_pipeline;
        req
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize, utoipa::ToSchema)]
pub struct AgentSettingsAdminPatch {
    pub composio: Option<ComposioSettingsAdminPatch>,
}

impl From<AgentSettingsAdminPatch> for AgentRuntimeSettingsPatch {
    fn from(value: AgentSettingsAdminPatch) -> Self {
        Self {
            proactive_enabled: None,
            proactive_cron: None,
            memory: None,
            composio: value.composio.map(Into::into),
            gmail: None,
        }
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize, utoipa::ToSchema)]
pub struct ComposioSettingsAdminPatch {
    pub api_key: Option<String>,
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

impl From<Settings> for RuntimeSettingsView {
    fn from(value: Settings) -> Self {
        Self {
            agent: AgentSettingsView {
                composio: ComposioSettingsView {
                    api_key: value.agent.composio.api_key,
                    entity_id: value.agent.composio.entity_id,
                },
            },
            job_pipeline: JobPipelineSettingsView {
                job_preferences: value.job_pipeline.job_preferences,
                score_threshold_auto: value.job_pipeline.score_threshold_auto,
                score_threshold_notify: value.job_pipeline.score_threshold_notify,
                resume_project_path: value.job_pipeline.resume_project_path,
            },
            updated_at: value.updated_at,
        }
    }
}
