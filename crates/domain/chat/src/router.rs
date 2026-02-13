//! HTTP API routes for the chat domain.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post, put},
};
use rara_sessions::types::{ChannelBinding, ChatMessage, SessionEntry, SessionKey};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::{error::ChatError, service::ChatService};

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    pub key:           String,
    pub title:         Option<String>,
    pub model:         Option<String>,
    pub system_prompt: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListSessionsQuery {
    pub limit:  Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct SendMessageRequest {
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct SendMessageResponse {
    pub message: ChatMessage,
}

#[derive(Debug, Deserialize)]
pub struct GetMessagesQuery {
    pub after_seq: Option<i64>,
    pub limit:     Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ForkSessionRequest {
    pub target_key:  String,
    pub fork_at_seq: i64,
}

#[derive(Debug, Deserialize)]
pub struct BindChannelRequest {
    pub channel_type: String,
    pub account:      String,
    pub chat_id:      String,
    pub session_key:  String,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Register all chat routes on a new router with shared state.
pub fn routes(service: ChatService) -> Router {
    Router::new()
        // Sessions
        .route("/api/v1/chat/sessions", post(create_session))
        .route("/api/v1/chat/sessions", get(list_sessions))
        .route("/api/v1/chat/sessions/{key}", get(get_session))
        .route("/api/v1/chat/sessions/{key}", delete(delete_session))
        // Messages
        .route("/api/v1/chat/sessions/{key}/send", post(send_message))
        .route(
            "/api/v1/chat/sessions/{key}/messages",
            get(get_messages),
        )
        .route(
            "/api/v1/chat/sessions/{key}/messages",
            delete(clear_messages),
        )
        // Fork
        .route(
            "/api/v1/chat/sessions/{key}/fork",
            post(fork_session),
        )
        // Channel bindings
        .route("/api/v1/chat/channel-bindings", put(bind_channel))
        .route(
            "/api/v1/chat/channel-bindings/{channel_type}/{account}/{chat_id}",
            get(get_channel_binding),
        )
        .with_state(service)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

#[instrument(skip(service, req))]
async fn create_session(
    State(service): State<ChatService>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<SessionEntry>), ChatError> {
    let key = SessionKey::from_raw(req.key);
    let session = service
        .create_session(key, req.title, req.model, req.system_prompt)
        .await?;
    Ok((StatusCode::CREATED, Json(session)))
}

#[instrument(skip(service))]
async fn list_sessions(
    State(service): State<ChatService>,
    Query(q): Query<ListSessionsQuery>,
) -> Result<Json<Vec<SessionEntry>>, ChatError> {
    let sessions = service.list_sessions(q.limit, q.offset).await?;
    Ok(Json(sessions))
}

#[instrument(skip(service))]
async fn get_session(
    State(service): State<ChatService>,
    Path(key): Path<String>,
) -> Result<Json<SessionEntry>, ChatError> {
    let session = service.get_session(&SessionKey::from_raw(key)).await?;
    Ok(Json(session))
}

#[instrument(skip(service))]
async fn delete_session(
    State(service): State<ChatService>,
    Path(key): Path<String>,
) -> Result<StatusCode, ChatError> {
    service.delete_session(&SessionKey::from_raw(key)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip(service, req))]
async fn send_message(
    State(service): State<ChatService>,
    Path(key): Path<String>,
    Json(req): Json<SendMessageRequest>,
) -> Result<Json<SendMessageResponse>, ChatError> {
    let message = service
        .send_message(&SessionKey::from_raw(key), req.text)
        .await?;
    Ok(Json(SendMessageResponse { message }))
}

#[instrument(skip(service))]
async fn get_messages(
    State(service): State<ChatService>,
    Path(key): Path<String>,
    Query(q): Query<GetMessagesQuery>,
) -> Result<Json<Vec<ChatMessage>>, ChatError> {
    let messages = service
        .get_messages(&SessionKey::from_raw(key), q.after_seq, q.limit)
        .await?;
    Ok(Json(messages))
}

#[instrument(skip(service))]
async fn clear_messages(
    State(service): State<ChatService>,
    Path(key): Path<String>,
) -> Result<StatusCode, ChatError> {
    service
        .clear_messages(&SessionKey::from_raw(key))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip(service, req))]
async fn fork_session(
    State(service): State<ChatService>,
    Path(key): Path<String>,
    Json(req): Json<ForkSessionRequest>,
) -> Result<(StatusCode, Json<SessionEntry>), ChatError> {
    let forked = service
        .fork_session(
            &SessionKey::from_raw(key),
            SessionKey::from_raw(req.target_key),
            req.fork_at_seq,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(forked)))
}

#[instrument(skip(service, req))]
async fn bind_channel(
    State(service): State<ChatService>,
    Json(req): Json<BindChannelRequest>,
) -> Result<Json<ChannelBinding>, ChatError> {
    let binding = service
        .bind_channel(
            req.channel_type,
            req.account,
            req.chat_id,
            SessionKey::from_raw(req.session_key),
        )
        .await?;
    Ok(Json(binding))
}

#[instrument(skip(service))]
async fn get_channel_binding(
    State(service): State<ChatService>,
    Path((channel_type, account, chat_id)): Path<(String, String, String)>,
) -> Result<Json<Option<ChannelBinding>>, ChatError> {
    let binding = service
        .get_channel_session(&channel_type, &account, &chat_id)
        .await?;
    Ok(Json(binding))
}
