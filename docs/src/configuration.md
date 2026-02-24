# Configuration

Rara uses a two-tier configuration model:

- **Static config** (`AppConfig`) — immutable after startup, loaded from Infisical + environment variables
- **Runtime settings** (`Settings`) — mutable at runtime via REST API, persisted in PostgreSQL KV store

## Static Configuration

### Source Priority

Sources are evaluated in order (highest priority wins):

| Priority | Source | Description |
|----------|--------|-------------|
| 1 (highest) | `RARA__` environment variables | Direct env vars with `__` as nesting separator |
| 2 | Infisical | Secrets fetched from Infisical at startup (opt-in) |
| 3 (lowest) | Code defaults | Hardcoded default values |

### Infisical Integration

[Infisical](https://infisical.com) is the recommended way to manage static configuration and secrets. Rara integrates Infisical as a native config source via the official Rust SDK.

#### How It Works

```mermaid
sequenceDiagram
    participant App as Rara App
    participant Inf as Infisical
    participant Env as Environment

    App->>Env: Check INFISICAL_CLIENT_ID
    alt Infisical configured
        App->>Inf: Authenticate (Universal Auth)
        Inf-->>App: Access token
        App->>Inf: List secrets (project/env/path)
        Inf-->>App: Key-value pairs
        App->>App: Merge: env vars > Infisical > defaults
    else Not configured
        App->>App: Use env vars + defaults only
    end
    App->>App: Deserialize into AppConfig
```

At startup, `AppConfig::new()` optionally connects to Infisical using Universal Auth. Secrets are fetched and merged as a config source — env vars always take precedence.

#### Secret Key Mapping

Infisical secret names are converted to nested config keys using `__` as separator:

| Infisical Secret Key | Config Field |
|---------------------|-------------|
| `DATABASE__DATABASE_URL` | `database.database_url` |
| `HTTP__BIND_ADDRESS` | `http.bind_address` |
| `OBJECT_STORE__ACCESS_KEY` | `object_store.access_key` |

Keys are lowercased automatically. This follows the same convention as the `RARA__` environment variables (minus the prefix).

#### Setup

1. **Create a Machine Identity** in your Infisical project (Settings → Machine Identities)
2. **Add Universal Auth** credentials to the identity
3. **Set bootstrap env vars** (in `.env` or actual environment):

```bash
# Required — presence of CLIENT_ID activates Infisical loading
INFISICAL_CLIENT_ID=your-client-id
INFISICAL_CLIENT_SECRET=your-client-secret
INFISICAL_PROJECT_ID=your-project-id

# Optional (with defaults)
INFISICAL_BASE_URL=https://app.infisical.com   # or your self-hosted instance
INFISICAL_ENVIRONMENT=dev                       # dev / staging / prod
INFISICAL_SECRET_PATH=/                         # secret folder path
```

4. **Add secrets** in the Infisical dashboard. Use `__` to denote nesting:

```
DATABASE__DATABASE_URL = postgres://user:pass@db:5432/rara
DATABASE__MAX_CONNECTIONS = 20
HTTP__BIND_ADDRESS = 0.0.0.0:25555
OBJECT_STORE__ENDPOINT = http://minio:9000
OBJECT_STORE__ACCESS_KEY = admin
OBJECT_STORE__SECRET_KEY = supersecret
```

#### Fail-Open Behavior

- If `INFISICAL_CLIENT_ID` is **not set**, Infisical is silently skipped
- If Infisical is **unreachable**, startup continues with a warning log — env vars and defaults still work
- This ensures local development works with just `.env` or env vars, no Infisical required

#### Kubernetes Deployment

For K8s, two approaches work:

| Approach | How | When to Use |
|----------|-----|-------------|
| **SDK (app-level)** | Set `INFISICAL_*` env vars in the Pod | Simple, works everywhere |
| **K8s Operator** | Infisical Operator syncs secrets → K8s Secrets → Pod env vars | Zero app awareness, GitOps-friendly |

The K8s operator is deployed via `rara-infra` Helm charts (`infisical-standalone` + `secrets-operator`). When using the operator, the app sees secrets as regular environment variables — no SDK involved.

### All Config Keys

#### Database (`database.*`)

| Key | Env Var | Default | Description |
|-----|---------|---------|-------------|
| `database_url` | `RARA__DATABASE__DATABASE_URL` | `postgres://postgres:postgres@localhost:5432/job` | PostgreSQL connection string |
| `migration_dir` | `RARA__DATABASE__MIGRATION_DIR` | `crates/rara-model/migrations` | SQLx migration directory |
| `max_connections` | `RARA__DATABASE__MAX_CONNECTIONS` | `10` | Connection pool max size |
| `min_connections` | `RARA__DATABASE__MIN_CONNECTIONS` | `1` | Connection pool min idle |
| `connect_timeout` | `RARA__DATABASE__CONNECT_TIMEOUT` | `30s` | Connection timeout |
| `max_lifetime` | `RARA__DATABASE__MAX_LIFETIME` | `1800s` | Max connection lifetime |
| `idle_timeout` | `RARA__DATABASE__IDLE_TIMEOUT` | `600s` | Idle connection timeout |

#### HTTP Server (`http.*`)

| Key | Env Var | Default | Description |
|-----|---------|---------|-------------|
| `bind_address` | `RARA__HTTP__BIND_ADDRESS` | `127.0.0.1:25555` | HTTP listen address |
| `max_body_size` | `RARA__HTTP__MAX_BODY_SIZE` | `100MB` | Max request body |
| `enable_cors` | `RARA__HTTP__ENABLE_CORS` | `true` | CORS headers |
| `request_timeout` | `RARA__HTTP__REQUEST_TIMEOUT` | `60` | Timeout in seconds |

#### gRPC Server (`grpc.*`)

| Key | Env Var | Default | Description |
|-----|---------|---------|-------------|
| `bind_address` | `RARA__GRPC__BIND_ADDRESS` | `127.0.0.1:50051` | gRPC listen address |
| `server_address` | `RARA__GRPC__SERVER_ADDRESS` | `127.0.0.1:50051` | Advertised address |
| `max_recv_message_size` | `RARA__GRPC__MAX_RECV_MESSAGE_SIZE` | `512MB` | Max receive message |
| `max_send_message_size` | `RARA__GRPC__MAX_SEND_MESSAGE_SIZE` | `512MB` | Max send message |

#### Object Store / MinIO (`object_store.*`)

| Key | Env Var | Default | Description |
|-----|---------|---------|-------------|
| `endpoint` | `RARA__OBJECT_STORE__ENDPOINT` | `http://localhost:9000` | S3 endpoint |
| `bucket` | `RARA__OBJECT_STORE__BUCKET` | `rara` | Bucket name |
| `access_key` | `RARA__OBJECT_STORE__ACCESS_KEY` | `minioadmin` | Access key ID |
| `secret_key` | `RARA__OBJECT_STORE__SECRET_KEY` | `minioadmin` | Secret access key |

#### Other

| Key | Env Var | Default | Description |
|-----|---------|---------|-------------|
| `main_service_http_base` | `RARA__MAIN_SERVICE_HTTP_BASE` | `http://127.0.0.1:25555` | Base URL for internal service calls |

---

## Runtime Settings

Runtime settings are stored in PostgreSQL (KV store) and can be changed at any time via the REST API without restarting the service.

**Endpoint**: `GET/PATCH /api/v1/settings`

Changes are broadcast to all subscribers in-process via a `watch` channel, taking effect immediately.

### Settings Groups

#### AI (`ai.*`)

| Field | Description |
|-------|-------------|
| `openrouter_api_key` | OpenRouter API key |
| `provider` | `"openrouter"` or `"ollama"` |
| `ollama_base_url` | Ollama API URL (default: `http://localhost:11434`) |
| `default_model` | Default model for all scenarios |
| `job_model` | Model for job analysis tasks |
| `chat_model` | Model for chat conversations |
| `favorite_models` | Pinned model IDs for UI |
| `chat_model_fallbacks` | Fallback chain for chat |
| `job_model_fallbacks` | Fallback chain for jobs |

#### Telegram (`telegram.*`)

| Field | Description |
|-------|-------------|
| `bot_token` | Telegram bot token |
| `chat_id` | Primary chat ID |
| `allowed_group_chat_id` | Allowed group chat |
| `notification_channel_id` | Channel for automated notifications |

#### Agent (`agent.*`)

| Field | Description |
|-------|-------------|
| `soul` | Agent personality prompt |
| `chat_system_prompt` | Custom system prompt for chat |
| `proactive_enabled` | Enable proactive messaging |
| `proactive_cron` | Cron schedule for proactive checks |
| `memory.chroma_url` | Chroma vector DB URL |
| `memory.chroma_collection` | Collection name |
| `memory.chroma_api_key` | Chroma API key |
| `composio.api_key` | Composio API key |
| `composio.entity_id` | Composio entity ID |
| `gmail.address` | Gmail sender address |
| `gmail.app_password` | Gmail app password |
| `gmail.auto_send_enabled` | Allow automatic email sending |

#### Job Pipeline (`job_pipeline.*`)

| Field | Description |
|-------|-------------|
| `job_preferences` | Target roles, tech stack (markdown) |
| `score_threshold_auto` | Auto-apply score threshold (default: 85) |
| `score_threshold_notify` | Notification threshold (default: 60) |
| `resume_project_path` | Local path to typst resume project |
| `pipeline_cron` | Cron for automatic pipeline runs |

### Bootstrap from Environment

Some runtime settings can be bootstrapped from environment variables on first startup:

| Env Var | Settings Field |
|---------|---------------|
| `TELEGRAM_BOT_TOKEN` | `telegram.bot_token` |
| `OPENROUTER_API_KEY` | `ai.openrouter_api_key` |

These are only used if the KV store has no existing value. After that, changes are made via the API.
