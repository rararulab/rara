# Rara

A self-evolving, developer-first personal proactive agent built in Rust.

Unlike generic AI assistants that wait for your commands, Rara proactively monitors your context, learns from interactions, and takes action on your behalf. Built with a kernel-inspired architecture, it's designed for developers who want an AI agent that grows with them.

## Highlights

- **Proactive** — Heartbeat-driven background actions, not just request-response
- **Self-evolving** — 3-layer memory (facts, notes, recall) + skills system that learns and adapts
- **Developer-first** — Deep integration with Git, K8s, coding workflows, workspace management
- **Multi-channel** — Telegram, Web Chat, Terminal interfaces
- **Kernel architecture** — OS-inspired event loop, process table, sessions, and approval guards

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        Channels                             │
│              Telegram  ·  WebChat  ·  Terminal               │
└──────────────────────────┬──────────────────────────────────┘
                           │
┌──────────────────────────▼──────────────────────────────────┐
│                        Kernel                               │
│  Event Loop  ·  Process Table  ·  Sessions  ·  Approval     │
│  LLM API  ·  Tool Registry  ·  Memory  ·  Guard  ·  Events │
└──┬───────────────┬───────────────┬──────────────────────────┘
   │               │               │
   ▼               ▼               ▼
┌────────┐  ┌────────────┐  ┌──────────────┐
│ Memory │  │   Skills   │  │  Extensions  │
│        │  │            │  │              │
│ mem0   │  │ discovery  │  │ git          │
│ Memos  │  │ registry   │  │ coding-task  │
│Hindsight│ │ install    │  │ workspace    │
│        │  │ watcher    │  │ k8s          │
└────────┘  └────────────┘  │ backend-admin│
                            └──────────────┘
   │               │               │
   ▼               ▼               ▼
┌─────────────────────────────────────────────────────────────┐
│                      Integrations                           │
│         MCP  ·  Composio  ·  OAuth  ·  Credential Store     │
└─────────────────────────────────────────────────────────────┘
```

### Crate Map

| Layer | Crates | Purpose |
|-------|--------|---------|
| **Entry** | `rara-cmd`, `rara-app` | CLI binary and application composition root |
| **Server** | `rara-server` | HTTP + gRPC endpoints |
| **Core** | `rara-kernel`, `rara-boot`, `rara-channels` | Agent kernel, bootstrap, channel adapters |
| **Capabilities** | `rara-memory`, `rara-skills`, `rara-sessions` | 3-layer memory, skill discovery/management, conversation persistence |
| **Extensions** | `rara-git`, `rara-coding-task`, `rara-workspace`, `rara-backend-admin` | Developer-focused agent capabilities |
| **Integrations** | `rara-mcp`, `rara-composio`, `rara-codex-oauth`, `rara-k8s` | External service adapters |
| **Foundation** | `base`, `rara-error`, `rara-paths`, `rara-model` | Shared primitives, error types, paths, data models |

## Getting Started

### Prerequisites

- Rust (see `rust-toolchain.toml` for version)
- PostgreSQL 17+
- Node.js 20+ (for web frontend)
- [just](https://github.com/casey/just) (task runner)

### Development

```bash
# install dependencies and check
just check

# run the server (HTTP + background workers)
just run

# run the web frontend
cd web && npm install && npm run dev

# format and lint
just fmt
just clippy
```

### Docker Compose (Containerized Agent)

```bash
# build image
just docker-agent-build

# start/update rara-agent server
just docker-agent-up

# run interactive chat mode in container
just docker-agent-chat

# cleanup compose stack, volumes and local images
just docker-agent-clean
```

`docker-compose.agent.yml` does not mount host project directories to the
`rara-agent` container. The runtime uses a read-only root filesystem + tmpfs
and resource limits (`cpus`, `mem_limit`, `pids_limit`).

### Configuration

Copy `env.local.example` to `.env` and configure:

- `DATABASE_URL` — PostgreSQL connection
- `TELEGRAM_BOT_TOKEN` / `TELEGRAM_CHAT_ID` — Telegram channel
- LLM provider API keys

## Tech Stack

- **Backend**: Rust, axum, tokio, sqlx, tonic (gRPC)
- **Frontend**: React 19, Tailwind v4, shadcn/ui, TanStack Query v5
- **Database**: PostgreSQL
- **Memory**: mem0 (facts) + Memos (notes) + Hindsight (recall/reflect)
- **Tools**: MCP protocol, Composio integration
- **Deploy**: Docker Compose, Helm chart (K8s)

## License

Apache-2.0
