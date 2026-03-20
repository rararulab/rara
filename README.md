# Rara

A self-evolving, developer-first personal proactive agent built in Rust.

Unlike generic AI assistants that wait for your commands, Rara proactively monitors your context, learns from interactions, and takes action on your behalf. Built with a kernel-inspired architecture, it's designed for developers who want an AI agent that grows with them.

## Highlights

- **Proactive** — Heartbeat-driven background actions, not just request-response
- **Tape memory** — Append-only JSONL tape with anchor-based context windowing, fork/merge for transactional turns
- **Developer-first** — Deep integration with Git, coding workflows, workspace management
- **Multi-channel** — Telegram, Web Chat, Terminal interfaces
- **Kernel architecture** — OS-inspired event loop with 6 core components: LLM, Tool, Memory, Session, Guard, EventBus

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                         Channels                              │
│               Telegram  ·  WebChat  ·  Terminal                │
└───────────────────────────┬──────────────────────────────────┘
                            │
┌───────────────────────────▼──────────────────────────────────┐
│                         Kernel                                │
│                                                               │
│   ┌─────────┐  ┌──────────┐  ┌──────────┐  ┌─────────────┐  │
│   │   LLM   │  │   Tool   │  │  Memory  │  │   Session    │  │
│   │ Driver  │  │ Registry │  │  (Tape)  │  │   Index      │  │
│   └─────────┘  └──────────┘  └──────────┘  └─────────────┘  │
│   ┌─────────┐  ┌──────────────────────────────────────────┐  │
│   │  Guard  │  │  EventBus · Notifications · Queue        │  │
│   │Pipeline │  │  (sharded event dispatch + pub/sub)      │  │
│   └─────────┘  └──────────────────────────────────────────┘  │
│                                                               │
│   Agent Loop · Context Budget · IO Subsystem · Rate Limiter  │
└───┬──────────────┬──────────────┬────────────────────────────┘
    │              │              │
    ▼              ▼              ▼
┌────────┐  ┌───────────┐  ┌──────────────┐
│  Tape  │  │   Skills  │  │  Extensions  │
│        │  │           │  │              │
│ JSONL  │  │ discovery │  │ git          │
│ append │  │ registry  │  │ backend-admin│
│ anchor │  │           │  │              │
│ fork   │  │           │  │              │
└────────┘  └───────────┘  └──────────────┘
    │              │              │
    ▼              ▼              ▼
┌──────────────────────────────────────────────────────────────┐
│                       Integrations                            │
│          MCP  ·  Composio  ·  OAuth  ·  Credential Store      │
└──────────────────────────────────────────────────────────────┘
```

### Tape Memory

The tape is the single source of truth for conversation history — an append-only JSONL file per session.

- **Entry types**: Message, ToolCall, ToolResult, Event, Anchor, Note, Summary
- **Anchors** enable context windowing — entries before an anchor can be trimmed from the LLM context, but the full tape remains searchable
- **Fork/merge** provides transactional turns — failed turns are discarded without polluting the tape
- **Two-layer context budget** keeps LLM context within limits (truncate large tool results, compress older outputs)

### Crate Map

| Layer | Crates | Purpose |
|-------|--------|---------|
| **Entry** | `rara-cmd`, `rara-app` | CLI binary and application composition root |
| **Server** | `rara-server` | HTTP + gRPC endpoints |
| **Core** | `rara-kernel`, `rara-channels`, `rara-agents`, `rara-soul` | Agent kernel, channel adapters, agent manifests, personality |
| **Capabilities** | `rara-skills`, `rara-sessions`, `rara-symphony` | Skill discovery, session persistence, issue orchestration |
| **Extensions** | `rara-git`, `rara-backend-admin` | Developer-focused agent capabilities |
| **Integrations** | `rara-mcp`, `rara-composio`, `rara-codex-oauth`, `rara-dock`, `rara-acp` | External service adapters, container orchestration, access control |
| **Infrastructure** | `rara-vault`, `rara-keyring-store`, `rara-pg-credential-store` | Secrets and credential management |
| **Foundation** | `base`, `rara-error`, `rara-runtime`, `rara-telemetry`, `rara-worker`, `rara-paths`, `rara-model`, `yunara-store`, `tool-macro`, `crawl4ai` | Shared primitives, error types, async runtimes, telemetry, task scheduling, paths, data models, KV store, proc macros |
| **Domain** | `rara-domain-shared` | Settings, identity, security primitives |

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

### Configuration

All configuration is loaded from YAML (`~/.config/rara/config.yaml`). Copy `config.example.yaml` and configure:

- LLM provider and model settings
- PostgreSQL connection
- Telegram bot token and chat ID
- Tape storage directory
- Kernel concurrency limits

## Tech Stack

- **Backend**: Rust, axum, tokio, sqlx, tonic (gRPC)
- **Frontend**: React 19, Tailwind v4, shadcn/ui, TanStack Query v5
- **Database**: PostgreSQL
- **Memory**: Tape system (append-only JSONL with anchor-based context windowing)
- **Tools**: MCP protocol, Composio integration
- **Deploy**: Docker Compose, Helm chart (K8s)

## License

Apache-2.0
