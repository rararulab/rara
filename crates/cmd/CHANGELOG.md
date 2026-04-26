# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.0.1] - 2026-04-26

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
- **ci**: Repair broken release pipeline ([#444](https://github.com/rararulab/rara/issues/444))
- **cmd**: Default chat user-id to first configured user ([#986](https://github.com/rararulab/rara/issues/986)) ([#987](https://github.com/rararulab/rara/issues/987))
- **kernel**: Use SessionKey/ChannelType types in ChannelMessage and ChannelBinding ([#1120](https://github.com/rararulab/rara/issues/1120)) ([#1132](https://github.com/rararulab/rara/issues/1132))
- **cmd**: Index 'rara debug' lookup via execution_traces SQL ([#1138](https://github.com/rararulab/rara/issues/1138)) ([#1139](https://github.com/rararulab/rara/issues/1139))
- **cmd**: Render full ExecutionTrace from SQL in 'rara debug' ([#1156](https://github.com/rararulab/rara/issues/1156)) ([#1161](https://github.com/rararulab/rara/issues/1161))
- **cmd**: Parse tool_call/tool_result payload correctly in debug timeline ([#1165](https://github.com/rararulab/rara/issues/1165)) ([#1169](https://github.com/rararulab/rara/issues/1169))
- **ci**: Fix rust 1.95 clippy lints ([#1667](https://github.com/rararulab/rara/issues/1667)) ([#1668](https://github.com/rararulab/rara/issues/1668))
- **store,kernel**: Root-cause SQLite writer contention causing silent trace loss ([#1843](https://github.com/rararulab/rara/issues/1843)) ([#1845](https://github.com/rararulab/rara/issues/1845))

### Documentation

- Update README to reflect tape-based architecture ([#783](https://github.com/rararulab/rara/issues/783)) ([#784](https://github.com/rararulab/rara/issues/784))
- Simplify readme with logo ([#1088](https://github.com/rararulab/rara/issues/1088)) ([#1089](https://github.com/rararulab/rara/issues/1089))
- Add inspired-by credits ([#1091](https://github.com/rararulab/rara/issues/1091)) ([#1092](https://github.com/rararulab/rara/issues/1092))

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
- **kernel**: Centralize loading hints with random selection ([#455](https://github.com/rararulab/rara/issues/455))
- **channels**: Add /status command with session info and scheduled jobs ([#450](https://github.com/rararulab/rara/issues/450)) ([#453](https://github.com/rararulab/rara/issues/453))
- **dock**: Generative UI canvas workbench ([#424](https://github.com/rararulab/rara/issues/424))
- **chat**: Support user image input in web and cli ([#475](https://github.com/rararulab/rara/issues/475))
- **gateway**: Send Telegram notification on shutdown ([#490](https://github.com/rararulab/rara/issues/490)) ([#491](https://github.com/rararulab/rara/issues/491))
- **kernel**: Implement pause_turn circuit breaker for agent loop ([#506](https://github.com/rararulab/rara/issues/506)) ([#508](https://github.com/rararulab/rara/issues/508))
- **kernel**: Show LLM reasoning for tool calls in progress display ([#661](https://github.com/rararulab/rara/issues/661)) ([#664](https://github.com/rararulab/rara/issues/664))
- **kernel**: Add tool call loop breaker ([#773](https://github.com/rararulab/rara/issues/773)) ([#775](https://github.com/rararulab/rara/issues/775))
- **kernel**: Stream bash stdout in real-time during tool execution ([#777](https://github.com/rararulab/rara/issues/777)) ([#788](https://github.com/rararulab/rara/issues/788))
- **channels**: Add WeChat iLink Bot channel adapter ([#827](https://github.com/rararulab/rara/issues/827)) ([#830](https://github.com/rararulab/rara/issues/830))
- **kernel**: Codex OAuth as first-class LLM provider ([#950](https://github.com/rararulab/rara/issues/950)) ([#953](https://github.com/rararulab/rara/issues/953))
- **web**: Align with rararulab/style ([#963](https://github.com/rararulab/rara/issues/963)) ([#964](https://github.com/rararulab/rara/issues/964))
- **login**: Replace callback server with paste-URL flow for codex OAuth ([#957](https://github.com/rararulab/rara/issues/957)) ([#960](https://github.com/rararulab/rara/issues/960))
- **cmd**: TUI-Telegram feature parity ([#961](https://github.com/rararulab/rara/issues/961)) ([#984](https://github.com/rararulab/rara/issues/984))
- **cmd**: Show thinking content in TUI like Claude Code ([#989](https://github.com/rararulab/rara/issues/989)) ([#990](https://github.com/rararulab/rara/issues/990))
- **cmd**: Rara setup interactive configuration wizard ([#1008](https://github.com/rararulab/rara/issues/1008)) ([#1011](https://github.com/rararulab/rara/issues/1011))
- **cmd**: Add `setup whisper` subcommand for standalone STT configuration ([#1059](https://github.com/rararulab/rara/issues/1059)) ([#1060](https://github.com/rararulab/rara/issues/1060))
- **app**: Auto-manage whisper-server lifecycle ([#1081](https://github.com/rararulab/rara/issues/1081)) ([#1082](https://github.com/rararulab/rara/issues/1082))
- **cmd**: Add 'rara debug <message_id>' CLI command ([#1135](https://github.com/rararulab/rara/issues/1135)) ([#1136](https://github.com/rararulab/rara/issues/1136))
- **cmd**: Add /name command to rename current session ([#1370](https://github.com/rararulab/rara/issues/1370)) ([#1371](https://github.com/rararulab/rara/issues/1371))
- **channels**: Telegram forum topics — auto-create, route, and delete ([#1430](https://github.com/rararulab/rara/issues/1430)) ([#1440](https://github.com/rararulab/rara/issues/1440))
- **channels**: Show model + context in TG pin floating preview ([#1541](https://github.com/rararulab/rara/issues/1541)) ([#1544](https://github.com/rararulab/rara/issues/1544))
- **sandbox**: Stage boxlite runtime ([#1699](https://github.com/rararulab/rara/issues/1699)) ([#1844](https://github.com/rararulab/rara/issues/1844))
- **telemetry**: Export OTLP traces to self-hosted Langfuse ([#1855](https://github.com/rararulab/rara/issues/1855)) ([#1859](https://github.com/rararulab/rara/issues/1859))
- **telemetry**: Integrate Pyroscope continuous profiling ([#1857](https://github.com/rararulab/rara/issues/1857)) ([#1862](https://github.com/rararulab/rara/issues/1862))

### Miscellaneous Tasks

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
- Add missing AGENT.md files for all crates ([#535](https://github.com/rararulab/rara/issues/535)) ([#539](https://github.com/rararulab/rara/issues/539))
- Remove symphony orchestrator ([#1432](https://github.com/rararulab/rara/issues/1432)) ([#1437](https://github.com/rararulab/rara/issues/1437))

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
- **kernel**: Fix KernelHandle API inconsistencies ([#1027](https://github.com/rararulab/rara/issues/1027)) ([#1031](https://github.com/rararulab/rara/issues/1031))
- Fix naming conventions and convention drift ([#1032](https://github.com/rararulab/rara/issues/1032)) ([#1035](https://github.com/rararulab/rara/issues/1035))
- **kernel**: Own trace build and save ([#1613](https://github.com/rararulab/rara/issues/1613)) ([#1614](https://github.com/rararulab/rara/issues/1614))
- **kernel**: Session-centric StreamHub event bus ([#1647](https://github.com/rararulab/rara/issues/1647)) ([#1652](https://github.com/rararulab/rara/issues/1652))
- **db**: Migrate sqlx consumers to diesel ([#1702](https://github.com/rararulab/rara/issues/1702)) ([#1737](https://github.com/rararulab/rara/issues/1737))
- **db**: Catch up with main and port new sqlx code ([#1744](https://github.com/rararulab/rara/issues/1744)) ([#1746](https://github.com/rararulab/rara/issues/1746))
- **db**: Third catchup with main ([#1747](https://github.com/rararulab/rara/issues/1747)) ([#1751](https://github.com/rararulab/rara/issues/1751))

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
