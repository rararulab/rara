# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.0.1] - 2026-03-20

### Bug Fixes

- **notifications**: Avoid 500 when pgmq archive table is absent
- **settings**: Add pipeline_cron field to test initializers
- **shared**: Add missing recall_every_turn field in test initializer ([#322](https://github.com/rararulab/rara/issues/322))
- **kernel**: Use ModelRepo for runtime model resolution, read Telegram token from settings

### Documentation

- Update README to reflect tape-based architecture ([#783](https://github.com/rararulab/rara/issues/783)) ([#784](https://github.com/rararulab/rara/issues/784))

### Features

- Add runtime settings with hot reload for ai and telegram
- **settings**: Add updated timestamp and toast-based feedback
- **notifications**: Rebuild observability around pgmq queue semantics
- Improve settings UX and markdown preview flow
- **settings**: Per-scenario model configuration
- **agents**: Add proactive agent with personality (Agent Soul)
- **chat**: Make default system prompt configurable via settings ([#121](https://github.com/rararulab/rara/issues/121))
- **memory**: Integrate agent memory with pg/sqlite backends
- **telegram**: Add group chat support with mention-based triggering
- **chat**: Dynamic OpenRouter model list + favorites ([#151](https://github.com/rararulab/rara/issues/151))
- **tools**: Add screenshot tool with Playwright + Telegram photo sending ([#157](https://github.com/rararulab/rara/issues/157))
- **openapi**: Add OpenAPI support with utoipa + Swagger UI ([#159](https://github.com/rararulab/rara/issues/159))
- **openapi**: Annotate chat, job, typst, resume, settings routes ([#178](https://github.com/rararulab/rara/issues/178))
- **agents**: Model fallback chain ([#193](https://github.com/rararulab/rara/issues/193))
- Integrate composio
- **email**: Integrate lettre for Gmail sending ([#216](https://github.com/rararulab/rara/issues/216))
- **pipeline**: Implement job pipeline agent and service ([#217](https://github.com/rararulab/rara/issues/217))
- **pipeline**: Add cron scheduling for automatic pipeline runs ([#220](https://github.com/rararulab/rara/issues/220))
- **pipeline**: Add MCP tool support to pipeline agent
- **pipeline**: Send notifications to dedicated Telegram channel ([#227](https://github.com/rararulab/rara/issues/227))
- **pipeline**: Add report_pipeline_stats tool and recipient-based notify
- **contacts**: Add telegram contacts allowlist ([#232](https://github.com/rararulab/rara/issues/232))
- **ai**: Add Ollama provider support for local LLM inference ([#240](https://github.com/rararulab/rara/issues/240))
- **settings**: Add SSH public key API endpoint
- **settings**: Add llmfit model recommendations endpoint ([#256](https://github.com/rararulab/rara/issues/256))
- **settings**: Integrate Ollama model management endpoints ([#268](https://github.com/rararulab/rara/issues/268))
- **web**: Add capability-based filtering to Ollama model selector ([#274](https://github.com/rararulab/rara/issues/274))
- Modularize settings admin routes
- **memory**: Add post-compaction recall and per-turn recall config ([#319](https://github.com/rararulab/rara/issues/319))
- **kernel**: Settings-driven SandboxConfig + hot reload ([#453](https://github.com/rararulab/rara/issues/453))
- **kernel**: Wire IngressRateLimiter into IOSubsystem resolve path

### Miscellaneous Tasks

- Establish job backend baseline
- Fmt code
- Format
- Rename to rara
- Format & some improvement & prompt markdown
- Change default HTTP port from 3000 to 25555
- Remove unused ollama-rs dependency
- Remove legacy domain settings router file
- Format
- Format
- Rustfmt formatting pass, fix Helm replicas/workers from `true` to `1`
- Add missing AGENT.md files for all crates ([#535](https://github.com/rararulab/rara/issues/535)) ([#539](https://github.com/rararulab/rara/issues/539))

### Refactor

- Rename job-domain-core to job-domain-shared and clean up unused types
- Merge convert.rs into types.rs and remove redundant tests
- Move TelegramService to shared crate, add notify-driven workers
- Decouple telegram bot with grpc/http boundary ([#94](https://github.com/rararulab/rara/issues/94))
- Move TelegramService into telegram-bot crate, add outbox pattern
- Remove notify domain crate, replace with lightweight shared notify client
- Realign domain boundaries and rename domain crates
- **notify**: Route observability through NotifyClient
- Unify runtime state into AppState with init() and routes()
- Extract crawl4ai into common crate and remove unused downloader
- **message-bus**: Rewrite as Coordinator + Command trait
- **memory**: Use Chroma server-side embeddings, remove HashEmbedder
- **memory**: Simplify to PG-only backend with required Chroma
- Add keyring-store crate, process group utils, layer READMEs, and dep upgrades
- **pipeline**: Move agent prompt into extension crate
- **settings**: Replace llmfit subprocess with llmfit-core git dependency ([#258](https://github.com/rararulab/rara/issues/258))
- **agent-core**: Centralize model configuration behind ModelRepo trait ([#279](https://github.com/rararulab/rara/issues/279))
- Remove compose_with_soul/resolve_soul and settings prompt fields ([#281](https://github.com/rararulab/rara/issues/281))
- Split settings admin domains and reorganize settings UI
- **backend-admin**: Move all domain routers into backend-admin ([#295](https://github.com/rararulab/rara/issues/295))
- Move contacts to telegram-bot, add ContactLookup trait ([#307](https://github.com/rararulab/rara/issues/307))
- **settings**: Move SettingsSvc + ollama from domain/shared to backend-admin ([#310](https://github.com/rararulab/rara/issues/310))
- **memory**: Integrate new MemoryManager into tools, orchestrator, and settings ([#313](https://github.com/rararulab/rara/issues/313))
- **codex**: Move oauth core logic out of backend-admin
- **codex**: Move oauth core into integrations crate
- Remove legacy proactive agent and agent scheduler
- Remove job pipeline module and related code
- **settings**: Unify runtime settings into flat KV store ([#401](https://github.com/rararulab/rara/issues/401))
- **kernel**: Consolidate queue/KV/LLM subsystems, remove rara-queue crate
- Remove PGMQ NotifyClient — replaced by kernel egress
- **llm**: Per-provider default_model and fallback_models ([#47](https://github.com/rararulab/rara/issues/47))
- **app**: Align knowledge config with settings-first architecture

<!-- generated by git-cliff -->
