//! HTTP API routes for the chat domain.
//!
//! All endpoints live under `/api/v1/chat/` and use JSON request/response
//! bodies. The router is constructed via [`routes`] and expects a
//! [`ChatService`] as shared axum state.
//!
//! ## Route table
//!
//! | Method   | Path                                                 | Description            |
//! |----------|------------------------------------------------------|------------------------|
//! | `POST`   | `/api/v1/chat/sessions`                              | Create a session       |
//! | `GET`    | `/api/v1/chat/sessions`                              | List sessions          |
//! | `GET`    | `/api/v1/chat/sessions/{key}`                        | Get a session          |
//! | `DELETE` | `/api/v1/chat/sessions/{key}`                        | Delete a session       |
//! | `POST`   | `/api/v1/chat/sessions/{key}/send`                   | Send a message         |
//! | `GET`    | `/api/v1/chat/sessions/{key}/messages`               | Get message history    |
//! | `DELETE` | `/api/v1/chat/sessions/{key}/messages`               | Clear messages         |
//! | `POST`   | `/api/v1/chat/sessions/{key}/fork`                   | Fork a session         |
//! | `PUT`    | `/api/v1/chat/channel-bindings`                      | Bind a channel         |
//! | `GET`    | `/api/v1/chat/channel-bindings/{type}/{account}/{id}`| Get channel binding    |

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

/// Request body for `POST /sessions`.
#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    /// Session key (e.g. `"user:alice"` or `"dm:alice:bob"`).
    pub key:           String,
    /// Optional human-readable title.
    pub title:         Option<String>,
    /// Optional LLM model override (e.g. `"gpt-4o"`).
    pub model:         Option<String>,
    /// Optional system prompt override.
    pub system_prompt: Option<String>,
}

/// Query parameters for `GET /sessions`.
#[derive(Debug, Deserialize)]
pub struct ListSessionsQuery {
    /// Maximum number of sessions to return (default: 50).
    pub limit:  Option<i64>,
    /// Number of sessions to skip (default: 0).
    pub offset: Option<i64>,
}

/// Request body for `POST /sessions/{key}/send`.
#[derive(Debug, Deserialize)]
pub struct SendMessageRequest {
    /// The user's message text.
    pub text: String,
    /// Optional list of image URLs to include as multimodal content.
    #[serde(default)]
    pub image_urls: Option<Vec<String>>,
}

/// Response body for `POST /sessions/{key}/send`.
#[derive(Debug, Serialize)]
pub struct SendMessageResponse {
    /// The persisted assistant response message.
    pub message: ChatMessage,
}

/// Query parameters for `GET /sessions/{key}/messages`.
#[derive(Debug, Deserialize)]
pub struct GetMessagesQuery {
    /// Only return messages with `seq > after_seq` (cursor-based pagination).
    pub after_seq: Option<i64>,
    /// Maximum number of messages to return.
    pub limit:     Option<i64>,
}

/// Request body for `POST /sessions/{key}/fork`.
#[derive(Debug, Deserialize)]
pub struct ForkSessionRequest {
    /// Key for the newly created forked session.
    pub target_key:  String,
    /// Fork point — messages with `seq <= fork_at_seq` are copied.
    pub fork_at_seq: i64,
}

/// Request body for `PUT /channel-bindings`.
#[derive(Debug, Deserialize)]
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

/// Build an axum [`Router`] with all chat endpoints and the given
/// [`ChatService`] as shared state.
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

/// `POST /api/v1/chat/sessions` — create a new session.
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

/// `GET /api/v1/chat/sessions` — list sessions with pagination.
#[instrument(skip(service))]
async fn list_sessions(
    State(service): State<ChatService>,
    Query(q): Query<ListSessionsQuery>,
) -> Result<Json<Vec<SessionEntry>>, ChatError> {
    let sessions = service.list_sessions(q.limit, q.offset).await?;
    Ok(Json(sessions))
}

/// `GET /api/v1/chat/sessions/{key}` — get a single session.
#[instrument(skip(service))]
async fn get_session(
    State(service): State<ChatService>,
    Path(key): Path<String>,
) -> Result<Json<SessionEntry>, ChatError> {
    let session = service.get_session(&SessionKey::from_raw(key)).await?;
    Ok(Json(session))
}

/// `DELETE /api/v1/chat/sessions/{key}` — delete a session and all its data.
#[instrument(skip(service))]
async fn delete_session(
    State(service): State<ChatService>,
    Path(key): Path<String>,
) -> Result<StatusCode, ChatError> {
    service.delete_session(&SessionKey::from_raw(key)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/v1/chat/sessions/{key}/send` — send a user message and receive
/// the assistant's response (synchronous, blocks until the agent loop
/// completes).
#[instrument(skip(service, req))]
async fn send_message(
    State(service): State<ChatService>,
    Path(key): Path<String>,
    Json(req): Json<SendMessageRequest>,
) -> Result<Json<SendMessageResponse>, ChatError> {
    let message = service
        .send_message(&SessionKey::from_raw(key), req.text, req.image_urls)
        .await?;
    Ok(Json(SendMessageResponse { message }))
}

/// `GET /api/v1/chat/sessions/{key}/messages` — retrieve conversation
/// history with optional cursor-based pagination.
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

/// `DELETE /api/v1/chat/sessions/{key}/messages` — clear all messages for a
/// session (keeps the session itself).
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

/// `POST /api/v1/chat/sessions/{key}/fork` — fork a session at a specific
/// message sequence number.
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

/// `PUT /api/v1/chat/channel-bindings` — bind an external channel to a
/// session (upsert).
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

/// `GET /api/v1/chat/channel-bindings/{type}/{account}/{chat_id}` — resolve
/// a channel binding to its session.
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
