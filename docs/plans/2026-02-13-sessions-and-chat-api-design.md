# Sessions Crate & Chat HTTP API Design

## Overview

Implement a standalone `crates/sessions/` crate for persisting AI agent conversation sessions in PostgreSQL, and integrate it with the chat domain crate to expose HTTP chat endpoints. This decouples the chat interface from Telegram, allowing any client (web, CLI, Telegram bot) to interact with the AI agent through a unified HTTP API.

## Architecture

```
HTTP Client (Web / Telegram / CLI)
        │
        ▼
  chat domain crate (router.rs)
    POST /api/v1/chat/sessions              — create session
    GET  /api/v1/chat/sessions              — list sessions
    GET  /api/v1/chat/sessions/:key         — get session
    DELETE /api/v1/chat/sessions/:key       — delete session
    POST /api/v1/chat/sessions/:key/send    — send message (sync, runs agent loop)
    GET  /api/v1/chat/sessions/:key/messages — get history
    DELETE /api/v1/chat/sessions/:key/messages — clear history
    POST /api/v1/chat/sessions/:key/fork    — fork session at message index
    PUT  /api/v1/chat/channel-bindings      — bind channel to session
    GET  /api/v1/chat/channel-bindings/:type/:account/:chat — get active session
        │
        ▼
  chat domain crate (service.rs) — orchestration
    │                    │
    ▼                    ▼
  sessions crate       agents crate
  (session persistence) (agent loop + tools)
    │
    ▼
  PostgreSQL (chat_sessions, chat_messages, channel_session_bindings)
```

### Layer responsibilities

- **sessions crate** — Pure storage. Types, repository trait, PG implementation, migrations. No knowledge of agents or LLM.
- **chat domain crate** — Orchestration. Receives HTTP requests, manages session lifecycle, invokes agent runner with session history, persists results.
- **agents crate** — Stateless LLM execution. Accepts history as `Vec<Message>`, returns response. No persistence.

## Sessions Crate (`crates/sessions/`)

### File structure

```
crates/sessions/
├── Cargo.toml
├── migrations/
│   └── 20260213000000_chat_sessions_init.sql
└── src/
    ├── lib.rs
    ├── types.rs
    ├── error.rs
    ├── repository.rs
    └── pg_repository.rs
```

### Types (`types.rs`)

```rust
/// Structured session key: "agent:{id}:main" or "agent:{id}:channel:{ch}:account:{acct}:peer:{kind}:{id}"
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionKey(pub String);

impl SessionKey {
    pub fn main(agent_id: &str) -> Self;
    pub fn for_peer(agent_id: &str, channel: &str, account: &str, peer_kind: &str, peer_id: &str) -> Self;
}

/// DM scope mode for session key generation.
#[derive(Debug, Clone)]
pub enum DmScope {
    Main,
    PerPeer,
    PerChannelPeer,
    PerAccountChannelPeer,
}

/// Session metadata row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    pub id: Uuid,
    pub session_key: String,
    pub label: String,
    pub model: Option<String>,
    pub preview: Option<String>,
    pub message_count: i32,
    pub last_seen_message_count: i32,
    pub archived: bool,
    pub parent_session_key: Option<String>,
    pub fork_point: Option<i32>,
    pub version: i32,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

/// Persisted chat message. Tagged by role for serde.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum ChatMessage {
    System {
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        created_at: Option<chrono::DateTime<Utc>>,
    },
    User {
        content: MessageContent,
        #[serde(skip_serializing_if = "Option::is_none")]
        created_at: Option<chrono::DateTime<Utc>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        channel: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        run_id: Option<Uuid>,
    },
    Assistant {
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        created_at: Option<chrono::DateTime<Utc>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        input_tokens: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        output_tokens: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        run_id: Option<Uuid>,
    },
    Tool {
        tool_call_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        created_at: Option<chrono::DateTime<Utc>>,
    },
    ToolResult {
        tool_call_id: String,
        tool_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<serde_json::Value>,
        success: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        created_at: Option<chrono::DateTime<Utc>>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Multimodal(Vec<ContentBlock>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    ImageUrl { url: String },
}

/// Channel-to-session binding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelBinding {
    pub channel_type: String,
    pub account_id: String,
    pub chat_id: String,
    pub session_key: String,
    pub created_at: chrono::DateTime<Utc>,
}
```

### Repository trait (`repository.rs`)

```rust
#[async_trait]
pub trait SessionRepository: Send + Sync {
    // Session CRUD
    async fn create_session(&self, key: &str, label: &str) -> Result<SessionEntry>;
    async fn get_session(&self, key: &str) -> Result<Option<SessionEntry>>;
    async fn list_sessions(&self, archived: bool) -> Result<Vec<SessionEntry>>;
    async fn delete_session(&self, key: &str) -> Result<bool>;
    async fn update_label(&self, key: &str, label: &str) -> Result<()>;
    async fn update_model(&self, key: &str, model: &str) -> Result<()>;
    async fn set_preview(&self, key: &str, preview: Option<&str>) -> Result<()>;
    async fn set_archived(&self, key: &str, archived: bool) -> Result<()>;
    async fn touch(&self, key: &str, message_count: i32) -> Result<()>;
    async fn fork_session(&self, source_key: &str, new_key: &str, fork_point: i32) -> Result<SessionEntry>;
    async fn list_children(&self, parent_key: &str) -> Result<Vec<SessionEntry>>;

    // Messages
    async fn append_message(&self, session_key: &str, message: &ChatMessage) -> Result<i32>; // returns seq
    async fn read_messages(&self, session_key: &str) -> Result<Vec<ChatMessage>>;
    async fn read_last_n_messages(&self, session_key: &str, n: i32) -> Result<Vec<ChatMessage>>;
    async fn clear_messages(&self, session_key: &str) -> Result<()>;
    async fn message_count(&self, session_key: &str) -> Result<i32>;
    async fn replace_history(&self, session_key: &str, messages: &[ChatMessage]) -> Result<()>;

    // Channel bindings
    async fn bind_channel(&self, channel_type: &str, account_id: &str, chat_id: &str, session_key: &str) -> Result<()>;
    async fn get_active_session(&self, channel_type: &str, account_id: &str, chat_id: &str) -> Result<Option<String>>;
    async fn unbind_channel(&self, channel_type: &str, account_id: &str, chat_id: &str) -> Result<bool>;
}
```

### PostgreSQL Implementation (`pg_repository.rs`)

Implements `SessionRepository` using `sqlx::PgPool`. Messages are stored as individual rows in `chat_messages` with a per-session `seq` counter. The `content` column stores JSONB for flexible content types.

### Migration (`migrations/20260213000000_chat_sessions_init.sql`)

```sql
CREATE TABLE chat_sessions (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_key             TEXT NOT NULL UNIQUE,
    label                   TEXT NOT NULL DEFAULT '',
    model                   TEXT,
    preview                 TEXT,
    message_count           INTEGER NOT NULL DEFAULT 0,
    last_seen_message_count INTEGER NOT NULL DEFAULT 0,
    archived                BOOLEAN NOT NULL DEFAULT FALSE,
    parent_session_key      TEXT REFERENCES chat_sessions(session_key) ON DELETE SET NULL,
    fork_point              INTEGER,
    version                 INTEGER NOT NULL DEFAULT 0,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE chat_messages (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_key     TEXT NOT NULL REFERENCES chat_sessions(session_key) ON DELETE CASCADE,
    seq             INTEGER NOT NULL,
    role            TEXT NOT NULL,
    content         JSONB NOT NULL,
    model           TEXT,
    provider        TEXT,
    input_tokens    INTEGER,
    output_tokens   INTEGER,
    run_id          UUID,
    tool_call_id    TEXT,
    tool_name       TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(session_key, seq)
);
CREATE INDEX idx_chat_messages_session_seq ON chat_messages(session_key, seq);

CREATE TABLE channel_session_bindings (
    channel_type    TEXT NOT NULL,
    account_id      TEXT NOT NULL,
    chat_id         TEXT NOT NULL,
    session_key     TEXT NOT NULL REFERENCES chat_sessions(session_key) ON DELETE CASCADE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (channel_type, account_id, chat_id)
);
```

## Chat Domain Crate Updates

### Service refactor (`service.rs`)

The `ChatService` will be refactored to orchestrate sessions + agent runner:

```rust
pub struct ChatService {
    session_repo: Arc<dyn SessionRepository>,
    agent_loader: OpenRouterLoaderRef,
    default_model: String,
    default_system_prompt: String,
}

impl ChatService {
    /// Send a message in a session. Runs the agent loop and returns the assistant response.
    pub async fn send_message(&self, session_key: &str, request: SendMessageRequest) -> Result<SendMessageResponse>;

    /// Create a new session.
    pub async fn create_session(&self, request: CreateSessionRequest) -> Result<SessionEntry>;

    /// List sessions.
    pub async fn list_sessions(&self, archived: bool) -> Result<Vec<SessionEntry>>;

    /// Get session details.
    pub async fn get_session(&self, key: &str) -> Result<SessionEntry>;

    /// Delete a session and all its messages.
    pub async fn delete_session(&self, key: &str) -> Result<()>;

    /// Get message history.
    pub async fn get_messages(&self, key: &str) -> Result<Vec<ChatMessage>>;

    /// Clear message history.
    pub async fn clear_messages(&self, key: &str) -> Result<()>;

    /// Fork a session at a given message index.
    pub async fn fork_session(&self, key: &str, fork_point: i32, new_label: &str) -> Result<SessionEntry>;

    /// Bind a channel to a session.
    pub async fn bind_channel(&self, binding: ChannelBindingRequest) -> Result<()>;

    /// Get active session for a channel.
    pub async fn get_channel_session(&self, channel_type: &str, account_id: &str, chat_id: &str) -> Result<Option<String>>;
}
```

### `send_message` flow

1. Load session (create if not exists)
2. Load message history from DB
3. Persist user message
4. Convert history to `Vec<openrouter_rs::api::chat::Message>`
5. Build `AgentRunner` with system prompt + history + user content
6. Run agent loop (with tools if registered)
7. Extract assistant response text
8. Persist assistant message
9. Update session metadata (message_count, preview, updated_at)
10. Return response

### Routes (`router.rs`)

All routes under `/api/v1/chat/`:
- `POST /sessions` — create session
- `GET /sessions` — list sessions
- `GET /sessions/:key` — get session
- `DELETE /sessions/:key` — delete session
- `POST /sessions/:key/send` — send message
- `GET /sessions/:key/messages` — get history
- `DELETE /sessions/:key/messages` — clear history
- `POST /sessions/:key/fork` — fork session
- `PUT /channel-bindings` — bind channel
- `GET /channel-bindings/:type/:account/:chat` — get channel session

## Testing

- Sessions crate: testcontainers PostgreSQL for repository tests
- Chat domain: integration tests with real DB + mock agent (or skip agent call in tests)
- Unit tests for type serialization/deserialization

## Implementation Order

1. Sessions crate: types.rs, error.rs, repository.rs, pg_repository.rs, migrations, tests
2. Chat domain: refactor service.rs, new routes, integration
3. Workspace: add sessions to Cargo.toml members, wire in app composition root
