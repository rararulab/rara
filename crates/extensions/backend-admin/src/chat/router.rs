// Copyright 2025 Rararulab
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

//! HTTP API routes for the chat domain.
//!
//! All endpoints live under `/api/v1/chat/` and use JSON request/response
//! bodies. The router is constructed via [`routes`] and expects a
//! [`SessionService`] as shared axum state.
//!
//! ## Route table
//!
//! | Method   | Path                                                 | Description            |
//! |----------|------------------------------------------------------|------------------------|
//! | `GET`    | `/api/v1/chat/models`                                | List available models  |
//! | `PUT`    | `/api/v1/chat/models/favorites`                      | Set favorite models    |
//! | `POST`   | `/api/v1/chat/sessions`                              | Create a session       |
//! | `GET`    | `/api/v1/chat/sessions`                              | List sessions          |
//! | `GET`    | `/api/v1/chat/sessions/{key}`                        | Get a session          |
//! | `PATCH`  | `/api/v1/chat/sessions/{key}`                        | Update session fields  |
//! | `DELETE` | `/api/v1/chat/sessions/{key}`                        | Delete a session       |
//! | `PUT`    | `/api/v1/chat/channel-bindings`                      | Bind a channel         |
//! | `GET`    | `/api/v1/chat/channel-bindings/{type}/{account}/{id}`| Get channel binding    |

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use rara_sessions::types::{ChannelBinding, SessionEntry, SessionKey};
use serde::Deserialize;
use tracing::instrument;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::chat::{error::ChatError, model_catalog::ChatModel, service::SessionService};

/// Parse a session key from a URL path parameter, returning 400 on invalid
/// UUID.
fn parse_session_key(raw: &str) -> Result<SessionKey, ChatError> {
    SessionKey::try_from_raw(raw).map_err(|_| ChatError::InvalidRequest {
        message: format!("invalid session key: {raw}"),
    })
}

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

/// Request body for `POST /sessions`.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateSessionRequest {
    /// Optional human-readable title.
    pub title:         Option<String>,
    /// Optional LLM model override (e.g. `"gpt-4o"`).
    pub model:         Option<String>,
    /// Optional system prompt override.
    pub system_prompt: Option<String>,
}

/// Query parameters for `GET /sessions`.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ListSessionsQuery {
    /// Maximum number of sessions to return (default: 50).
    pub limit:  Option<i64>,
    /// Number of sessions to skip (default: 0).
    pub offset: Option<i64>,
}

/// Request body for `PATCH /sessions/{key}`.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateSessionRequest {
    /// New human-readable title.
    pub title:         Option<String>,
    /// New LLM model identifier (e.g. `"openai/gpt-4o"`).
    pub model:         Option<String>,
    /// New system prompt override.
    pub system_prompt: Option<String>,
}

/// Request body for `PUT /models/favorites`.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SetFavoritesRequest {
    /// Model IDs to mark as favorites.
    pub model_ids: Vec<String>,
}

/// Request body for `PUT /channel-bindings`.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct BindChannelRequest {
    /// Channel type identifier (e.g. `"telegram"`, `"slack"`).
    pub channel_type: String,
    /// Account or bot identifier within the channel.
    pub account:      String,
    /// Chat or conversation identifier within the channel.
    pub chat_id:      String,
    /// Internal session key to bind to.
    pub session_key:  String,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Build an axum `Router` with all chat endpoints and the given
/// [`SessionService`] as shared state.
pub fn routes(service: SessionService) -> OpenApiRouter {
    model_routes(service.clone()).merge(session_routes(service))
}

fn model_routes(service: SessionService) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_models))
        .routes(routes!(set_favorites))
        .with_state(service)
}

fn session_routes(service: SessionService) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(create_session, list_sessions))
        .routes(routes!(get_session, update_session, delete_session))
        .routes(routes!(bind_channel))
        .routes(routes!(get_channel_binding))
        .with_state(service)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/chat/models` — return available LLM models.
///
/// When an OpenRouter API key is configured, this endpoint dynamically fetches
/// the full model list from OpenRouter (cached for 5 minutes). Without a key,
/// a curated fallback list is returned.
#[utoipa::path(
    get,
    path = "/api/v1/chat/models",
    tag = "chat",
    responses(
        (status = 200, description = "List of available models", body = Vec<ChatModel>),
    )
)]
async fn list_models(State(service): State<SessionService>) -> Json<Vec<ChatModel>> {
    let models = service.list_models().await;
    Json(models)
}

/// `PUT /api/v1/chat/models/favorites` — replace the user's favorite model
/// list.
#[utoipa::path(
    put,
    path = "/api/v1/chat/models/favorites",
    tag = "chat",
    request_body = SetFavoritesRequest,
    responses(
        (status = 200, description = "Updated favorite model IDs", body = Vec<String>),
    )
)]
#[instrument(skip(service, req))]
async fn set_favorites(
    State(service): State<SessionService>,
    Json(req): Json<SetFavoritesRequest>,
) -> Result<Json<Vec<String>>, ChatError> {
    let ids = req.model_ids;
    service.set_favorite_models(ids.clone()).await?;
    Ok(Json(ids))
}

/// `POST /api/v1/chat/sessions` — create a new session.
#[utoipa::path(
    post,
    path = "/api/v1/chat/sessions",
    tag = "chat",
    request_body = CreateSessionRequest,
    responses(
        (status = 201, description = "Session created", body = Object),
    )
)]
#[instrument(skip(service, req))]
async fn create_session(
    State(service): State<SessionService>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<SessionEntry>), ChatError> {
    let session = service
        .create_session(req.title, req.model, req.system_prompt)
        .await?;
    Ok((StatusCode::CREATED, Json(session)))
}

/// `GET /api/v1/chat/sessions` — list sessions with pagination.
#[utoipa::path(
    get,
    path = "/api/v1/chat/sessions",
    tag = "chat",
    params(
        ("limit" = Option<i64>, Query, description = "Maximum number of sessions to return"),
        ("offset" = Option<i64>, Query, description = "Number of sessions to skip"),
    ),
    responses(
        (status = 200, description = "List of sessions", body = Vec<Object>),
    )
)]
#[instrument(skip(service))]
async fn list_sessions(
    State(service): State<SessionService>,
    Query(q): Query<ListSessionsQuery>,
) -> Result<Json<Vec<SessionEntry>>, ChatError> {
    let sessions = service.list_sessions(q.limit, q.offset).await?;
    Ok(Json(sessions))
}

/// `GET /api/v1/chat/sessions/{key}` — get a single session.
#[utoipa::path(
    get,
    path = "/api/v1/chat/sessions/{key}",
    tag = "chat",
    params(("key" = String, Path, description = "Session key")),
    responses(
        (status = 200, description = "Session found", body = Object),
    )
)]
#[instrument(skip(service))]
async fn get_session(
    State(service): State<SessionService>,
    Path(key): Path<String>,
) -> Result<Json<SessionEntry>, ChatError> {
    let session = service.get_session(&parse_session_key(&key)?).await?;
    Ok(Json(session))
}

/// `PATCH /api/v1/chat/sessions/{key}` — partially update a session's
/// mutable fields (title, model, system_prompt).
#[utoipa::path(
    patch,
    path = "/api/v1/chat/sessions/{key}",
    tag = "chat",
    params(("key" = String, Path, description = "Session key")),
    request_body = UpdateSessionRequest,
    responses(
        (status = 200, description = "Session updated", body = Object),
    )
)]
#[instrument(skip(service, req))]
async fn update_session(
    State(service): State<SessionService>,
    Path(key): Path<String>,
    Json(req): Json<UpdateSessionRequest>,
) -> Result<Json<SessionEntry>, ChatError> {
    let session = service
        .update_session_fields(
            &parse_session_key(&key)?,
            req.title,
            req.model,
            req.system_prompt,
        )
        .await?;
    Ok(Json(session))
}

/// `DELETE /api/v1/chat/sessions/{key}` — delete a session and all its data.
#[utoipa::path(
    delete,
    path = "/api/v1/chat/sessions/{key}",
    tag = "chat",
    params(("key" = String, Path, description = "Session key")),
    responses(
        (status = 204, description = "Session deleted"),
    )
)]
#[instrument(skip(service))]
async fn delete_session(
    State(service): State<SessionService>,
    Path(key): Path<String>,
) -> Result<StatusCode, ChatError> {
    service.delete_session(&parse_session_key(&key)?).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `PUT /api/v1/chat/channel-bindings` — bind an external channel to a
/// session (upsert).
#[utoipa::path(
    put,
    path = "/api/v1/chat/channel-bindings",
    tag = "chat",
    request_body = BindChannelRequest,
    responses(
        (status = 200, description = "Channel binding created/updated", body = Object),
    )
)]
#[instrument(skip(service, req))]
async fn bind_channel(
    State(service): State<SessionService>,
    Json(req): Json<BindChannelRequest>,
) -> Result<Json<ChannelBinding>, ChatError> {
    let binding = service
        .bind_channel(
            req.channel_type,
            req.account,
            req.chat_id,
            parse_session_key(&req.session_key)?,
        )
        .await?;
    Ok(Json(binding))
}

/// `GET /api/v1/chat/channel-bindings/{type}/{account}/{chat_id}` — resolve
/// a channel binding to its session.
#[utoipa::path(
    get,
    path = "/api/v1/chat/channel-bindings/{channel_type}/{account}/{chat_id}",
    tag = "chat",
    params(
        ("channel_type" = String, Path, description = "Channel type (e.g. telegram, slack)"),
        ("account" = String, Path, description = "Account or bot identifier"),
        ("chat_id" = String, Path, description = "Chat or conversation identifier"),
    ),
    responses(
        (status = 200, description = "Channel binding found", body = Object),
    )
)]
#[instrument(skip(service))]
async fn get_channel_binding(
    State(service): State<SessionService>,
    Path((channel_type, account, chat_id)): Path<(String, String, String)>,
) -> Result<Json<Option<ChannelBinding>>, ChatError> {
    let binding = service
        .get_channel_session(&channel_type, &account, &chat_id)
        .await?;
    Ok(Json(binding))
}
