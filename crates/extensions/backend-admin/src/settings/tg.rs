use axum::{Json, extract::State, http::StatusCode};
use rara_domain_shared::settings::model::{Settings, TelegramRuntimeSettingsPatch, UpdateRequest};

use crate::settings::SettingsSvc;
use utoipa_axum::router::OpenApiRouter;

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct TgAdminSettingsView {
    pub configured: bool,
    pub chat_id: Option<i64>,
    pub allowed_group_chat_id: Option<i64>,
    pub notification_channel_id: Option<i64>,
    pub token_hint: Option<String>,
    #[schema(value_type = Option<String>)]
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, serde::Deserialize, utoipa::ToSchema)]
pub struct TgAdminUpdateRequest {
    pub bot_token: Option<String>,
    pub chat_id: Option<i64>,
    pub allowed_group_chat_id: Option<i64>,
    /// `None` = leave unchanged, `Some(None)` = clear, `Some(Some(id))` = set.
    pub notification_channel_id: Option<Option<i64>>,
}

pub(super) fn routes() -> OpenApiRouter<SettingsSvc> {
    OpenApiRouter::new().route(
        "/api/v1/tg/settings",
        axum::routing::get(get_tg_settings).put(update_tg_settings),
    )
}

#[utoipa::path(
    get,
    path = "/api/v1/tg/settings",
    tag = "telegram-admin",
    responses(
        (status = 200, description = "Telegram runtime settings", body = TgAdminSettingsView),
    )
)]
async fn get_tg_settings(State(state): State<SettingsSvc>) -> Json<TgAdminSettingsView> {
    Json(TgAdminSettingsView::from_settings(&state.current()))
}

#[utoipa::path(
    put,
    path = "/api/v1/tg/settings",
    tag = "telegram-admin",
    request_body = TgAdminUpdateRequest,
    responses(
        (status = 200, description = "Telegram settings updated", body = TgAdminSettingsView),
        (status = 500, description = "Internal server error"),
    )
)]
async fn update_tg_settings(
    State(state): State<SettingsSvc>,
    Json(req): Json<TgAdminUpdateRequest>,
) -> Result<Json<TgAdminSettingsView>, (StatusCode, String)> {
    let updated = state
        .update(UpdateRequest {
            telegram: Some(TelegramRuntimeSettingsPatch {
                bot_token: req.bot_token,
                chat_id: req.chat_id,
                allowed_group_chat_id: req.allowed_group_chat_id,
                notification_channel_id: req.notification_channel_id,
            }),
            ..UpdateRequest::default()
        })
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to update telegram settings: {e}"),
            )
        })?;

    Ok(Json(TgAdminSettingsView::from_settings(&updated)))
}

impl TgAdminSettingsView {
    fn from_settings(settings: &Settings) -> Self {
        let telegram = &settings.telegram;
        Self {
            configured: telegram.bot_token.is_some() && telegram.chat_id.is_some(),
            chat_id: telegram.chat_id,
            allowed_group_chat_id: telegram.allowed_group_chat_id,
            notification_channel_id: telegram.notification_channel_id,
            token_hint: secret_hint(telegram.bot_token.as_deref()),
            updated_at: settings.updated_at,
        }
    }
}

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
