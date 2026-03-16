# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.0.1] - 2026-03-16

### Bug Fixes

- **cmd**: Restore clap sub-command layout with server as core command
- **cmd**: Correct agents endpoint URL in rara top client
- **kernel**: Convert StreamEvent TextDelta/ReasoningDelta to struct variants ([#447](https://github.com/rararulab/rara/issues/447))
- **cmd**: Use centralized logs directory for server and gateway
- Tape memory
- **gateway**: Address code review feedback for telegram listener ([#199](https://github.com/rararulab/rara/issues/199))
- **gateway**: Remove /shutdown command to prevent unrecoverable state ([#199](https://github.com/rararulab/rara/issues/199))
- **gateway**: Add proxy + timeout support to gateway Telegram bot ([#205](https://github.com/rararulab/rara/issues/205))
- **channels**: Emit TextClear to fix tool progress notifications ([#207](https://github.com/rararulab/rara/issues/207))
- **telegram**: Harden tool-call XML stripping for streaming edge cases ([#314](https://github.com/rararulab/rara/issues/314))
- Resolve all clippy warnings across codebase ([#313](https://github.com/rararulab/rara/issues/313))

### Features

- Full-stack integration — DB repos, REST API, Docker Compose, app wiring
- Integrate saved jobs UI, MinIO config, and standalone telegram bot
- Integrate config crate for unified configuration management
- **telemetry**: Integrate Langfuse LLM observability via OTLP ([#253](https://github.com/rararulab/rara/issues/253))
- **infisical**: Add Infisical secrets manager integration ([#267](https://github.com/rararulab/rara/issues/267))
- Load app config from consul kv via rs-consul
- **web**: Unify agent chat, operations, and settings navigation
- **cli**: Add interactive chat command with TerminalAdapter ([#381](https://github.com/rararulab/rara/issues/381))
- **cmd**: Add `rara top` TUI subcommand for kernel observability ([#423](https://github.com/rararulab/rara/issues/423))
- **boot**: Channel binding-aware session resolver + misc fixes
- **kernel**: Enrich tool call traces with arguments and results ([#444](https://github.com/rararulab/rara/issues/444))
- **cmd**: Auto-enable OTLP tracing in Kubernetes environment ([#449](https://github.com/rararulab/rara/issues/449))
- **cmd**: Improve TUI session Gantt chart with metrics overlay and time axis
- **kernel**: Add group chat proactive reply with two-step LLM judgment ([#71](https://github.com/rararulab/rara/issues/71))
- **gateway**: Add supervisor foundation — spawn, health check, restart agent ([#92](https://github.com/rararulab/rara/issues/92))
- **gateway**: Add update detector — git fetch + rev comparison ([#93](https://github.com/rararulab/rara/issues/93))
- **gateway**: Expose admin HTTP API for restart, status, and shutdown ([#97](https://github.com/rararulab/rara/issues/97))
- **cmd**: Enable file logging by default and print lnav hint in gateway
- **gateway**: Wire update detector → executor → supervisor restart pipeline ([#102](https://github.com/rararulab/rara/issues/102))
- **gateway**: Send auto-update lifecycle notifications to Telegram channel ([#109](https://github.com/rararulab/rara/issues/109))
- **gateway**: Use sysinfo for rich system context in notifications ([#112](https://github.com/rararulab/rara/issues/112))
- **gateway**: Make commit rev a clickable GitHub link in notifications ([#114](https://github.com/rararulab/rara/issues/114))
- **channels**: Enrich tool-call progress with argument summaries ([#115](https://github.com/rararulab/rara/issues/115))
- **symphony**: Replace ralph sidecar with per-issue `ralph run` subprocess
- Align chat tui with openfang
- **tui**: Integrate kernel CommandHandler into chat TUI ([#194](https://github.com/rararulab/rara/issues/194))
- **gateway**: Move bot_token + notification_channel_id into GatewayConfig ([#199](https://github.com/rararulab/rara/issues/199))
- **gateway**: Wire Telegram listener into gateway startup and handle /shutdown ([#199](https://github.com/rararulab/rara/issues/199))
- **channels**: Render plan events in Telegram and Web adapters ([#251](https://github.com/rararulab/rara/issues/251))
- **gateway**: Wire ProcessMonitor into bootstrap and status endpoint ([#252](https://github.com/rararulab/rara/issues/252))
- **gateway**: Add /threshold and /stats Telegram commands ([#252](https://github.com/rararulab/rara/issues/252))
- **gateway**: Persist alert thresholds to gateway-state.yaml ([#266](https://github.com/rararulab/rara/issues/266))
- **channels**: Plan-execute TG 三级显示策略 + 单消息编辑流 ([#267](https://github.com/rararulab/rara/issues/267))
- **telegram**: Show input/output token counts in progress UX ([#304](https://github.com/rararulab/rara/issues/304))
- **kernel,telegram**: Rara_message_id end-to-end tracing and debug_trace tool ([#337](https://github.com/rararulab/rara/issues/337))
- **kernel**: Background agent spawning with proactive result delivery ([#340](https://github.com/rararulab/rara/issues/340))

### Refactor

- Rename rsketch crates to job
- Consolidate migrations, simplify job-source and app architecture
- Move TelegramService to shared crate, add notify-driven workers
- Decouple telegram bot with grpc/http boundary ([#94](https://github.com/rararulab/rara/issues/94))
- Realign domain boundaries and rename domain crates
- Unify runtime state into AppState with init() and routes()
- Extract crawl4ai into common crate and remove unused downloader
- Unify process model, integrate telegram-bot into app
- **infisical**: Implement config AsyncSource instead of env var injection ([#269](https://github.com/rararulab/rara/issues/269))
- Migrate all prompt consumers to PromptRepo + cleanup legacy code ([#278](https://github.com/rararulab/rara/issues/278))
- **kernel**: Remove InboundSink trait, use concrete IngressPipeline ([#398](https://github.com/rararulab/rara/issues/398))
- **kernel,config**: Rename SpawnTool → SyscallTool and harden Consul KV config
- **kernel**: Consolidate queue/KV/LLM subsystems, remove rara-queue crate
- Replace Consul with YAML config, continue SQLite migration, add session resumption
- **kernel**: Introduce EventBase for unified event metadata
- **cmd**: Replace Events tab with session-based view ([#46](https://github.com/rararulab/rara/issues/46))
- **kernel**: Session-centric runtime ([#48](https://github.com/rararulab/rara/issues/48))
- **cmd,server**: Update to session-centric naming ([#50](https://github.com/rararulab/rara/issues/50))
- Make it compile
- **kernel**: Decouple proactive judgment into GroupMessage event ([#79](https://github.com/rararulab/rara/issues/79))
- **gateway**: Use teloxide::Bot instead of raw reqwest for TG notifier ([#109](https://github.com/rararulab/rara/issues/109))
- **gateway**: Require Telegram config and enrich notification templates ([#109](https://github.com/rararulab/rara/issues/109))
- **gateway**: Rename notification_channel_id to chat_id ([#199](https://github.com/rararulab/rara/issues/199))

### Chore

- Establish job backend baseline
- Refactor
- Rename to rara
- Clean up stale code from prompt/agent refactoring ([#289](https://github.com/rararulab/rara/issues/289))
- Format
- Make lint pass across workspace
- Format
- Format
- Clean repo
- Format
- Clean
- Format
- Format chat tui files
- Format

### Chroe

- Foramt & add observable

<!-- generated by git-cliff -->

## [0.0.17] - 2026-01-20

<!-- generated by git-cliff -->

## [0.0.16] - 2026-01-20

### Documentation

- Add features section to README

### Chore

- Release v0.0.14
- Update remade
- Release v0.0.15

<!-- generated by git-cliff -->

## [0.0.15] - 2026-01-20

### Miscellaneous Tasks

- Release v0.0.14
- Update remade

<!-- generated by git-cliff -->

## [0.0.14] - 2026-01-20

### Miscellaneous Tasks

- Release v0.0.13

<!-- generated by git-cliff -->

## [0.0.13] - 2026-01-20

<!-- generated by git-cliff -->

## [0.0.12] - 2026-01-20

### Bug Fixes

- Ci
- Ci
- **clippy**: Resolve type conversion and code quality warnings
- Add version to internal workspace dependencies for crates.io publishing

### Features

- Integrate Buf for multi-language gRPC code generation
- Add worker abstraction and refactor common crate structure
- **worker**: Add fallible workers and pause modes
- **ci**: Add automated release workflow with cargo-dist

### Miscellaneous Tasks

- Enhance project configuration and CI setup
- Update project structure and dependencies
- Apply clippy lints and fix warnings across workspace
- Make clippy happy
- Release v0.0.11

### Refactor

- Optimize template project structure and cleanup

### Styling

- **cli**: Improve help text formatting

### Cmd

- Use global runtime for server command

<!-- generated by git-cliff -->

## [0.0.11] - 2026-01-20

### Bug Fixes

- Ci
- Ci
- **clippy**: Resolve type conversion and code quality warnings

### Features

- Integrate Buf for multi-language gRPC code generation
- Add worker abstraction and refactor common crate structure
- **worker**: Add fallible workers and pause modes
- **ci**: Add automated release workflow with cargo-dist

### Miscellaneous Tasks

- Enhance project configuration and CI setup
- Update project structure and dependencies
- Apply clippy lints and fix warnings across workspace
- Make clippy happy

### Refactor

- Optimize template project structure and cleanup

### Styling

- **cli**: Improve help text formatting

### Cmd

- Use global runtime for server command

<!-- generated by git-cliff -->
