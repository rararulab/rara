# Configuration

Rara uses a two-tier configuration model:

- **Static config** (`AppConfig`) — immutable after startup, loaded from Consul KV or environment variables
- **Runtime settings** (`Settings`) — mutable at runtime via REST API, persisted in PostgreSQL KV store

## Static Configuration

### Source Priority

When `CONSUL_HTTP_ADDR` is set, Consul KV is used as the sole config source (env vars are **not** read):

| Priority | Source | Description |
|----------|--------|-------------|
| 1 (highest) | Consul KV | Entries under `rara/config/` prefix, path segments map to nested keys |
| 2 (lowest) | Code defaults | Hardcoded default values |

When Consul is **not** configured (local dev fallback):

| Priority | Source | Description |
|----------|--------|-------------|
| 1 (highest) | `RARA__` environment variables | Direct env vars with `__` as nesting separator |
| 2 (lowest) | Code defaults | Hardcoded default values |

### Consul KV Integration

[Consul](https://www.consul.io/) KV is the recommended way to manage static configuration. Rara fetches config entries from the Consul HTTP API at startup.

#### How It Works

```mermaid
sequenceDiagram
    participant App as Rara App
    participant Consul as Consul KV
    participant Env as Environment

    App->>Env: Check CONSUL_HTTP_ADDR
    alt Consul configured
        App->>Consul: GET /v1/kv/rara/config/?recurse
        Consul-->>App: Base64-encoded KV entries
        App->>App: Decode + map paths to config keys
        App->>App: Merge: Consul KV > defaults
    else Not configured
        App->>App: Use RARA__ env vars + defaults only
    end
    App->>App: Deserialize into AppConfig
```

At startup, `AppConfig::new()` checks for `CONSUL_HTTP_ADDR`. If set, all config is loaded from Consul KV. Otherwise, `RARA__`-prefixed environment variables are used.

#### Key Mapping

Consul KV paths (after stripping the prefix) are converted to nested config keys using `/` as separator:

| Consul KV Path | Config Field |
|----------------|-------------|
| `rara/config/database/database_url` | `database.database_url` |
| `rara/config/http/bind_address` | `http.bind_address` |
| `rara/config/object_store/endpoint` | `object_store.endpoint` |

#### Setup

1. **Deploy Consul** via `rara-infra` Helm chart (included by default)
2. **Set bootstrap env vars** for the rara app:

```bash
# Required — presence of CONSUL_HTTP_ADDR activates Consul loading
CONSUL_HTTP_ADDR=http://rara-infra-consul-server:8500

# Optional
CONSUL_TOKEN=your-acl-token        # only if Consul ACLs are enabled
CONSUL_KV_PREFIX=rara/config/      # default prefix
```

3. **Seed config** into Consul KV (the `consul-kv-seed` Helm hook does this automatically):

```
rara/config/database/database_url = postgres://postgres:pass@db:5432/rara
rara/config/object_store/endpoint = http://minio:9000
rara/config/object_store/access_key_id = admin
rara/config/object_store/secret_access_key = supersecret
rara/config/object_store/bucket = rara
rara/config/memory/chroma_url = http://chromadb:8000
```

#### Fail-Open Behavior

- If `CONSUL_HTTP_ADDR` is **not set**, Consul is silently skipped and env vars are used
- If the Consul KV prefix has **no entries** (404), an empty config is returned and code defaults apply
- This ensures local development works with just `.env` or env vars, no Consul required

#### Kubernetes Deployment

In Kubernetes, the `consul-kv-seed` Helm post-install/post-upgrade hook automatically writes infrastructure credentials (database URL, MinIO keys, etc.) to Consul KV. The rara app Pod only needs `CONSUL_HTTP_ADDR` set as an environment variable.

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
