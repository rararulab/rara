# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.0.1] - 2026-03-24

### Bug Fixes

- Jobspy build improvements, Docker user paths, and justfile PYO3 config
- **routes**: Flatten OpenApiRouter merge chain to prevent stack overflow ([#180](https://github.com/rararulab/rara/issues/180))
- **app**: Connection pool timeout and Ctrl+C shutdown panic ([#296](https://github.com/rararulab/rara/issues/296))
- **kernel**: Use ModelRepo for runtime model resolution, read Telegram token from settings
- **boot,channels**: Session key mismatch — messages persisted to wrong session
- **gateway**: Use CancellationToken for shutdown + add docs and justfile updates ([#85](https://github.com/rararulab/rara/issues/85))
- **gateway**: Ctrl+C now reliably stops gateway during all phases
- **app**: Clean up stale staging worktrees on executor init ([#111](https://github.com/rararulab/rara/issues/111))
- **gateway**: Only trigger update when remote is ahead of local
- **config**: Ensure clean YAML roundtrip for AppConfig ([#121](https://github.com/rararulab/rara/issues/121))
- **security**: Enforce tool permissions in agent loop
- **kernel**: Auto-register system user for internal sessions ([#117](https://github.com/rararulab/rara/issues/117))
- **tools**: Rename all tool names to match OpenAI ^[a-zA-Z0-9-]+$ pattern
- Tape memory
- **mita**: Fix tape name mismatch and add session recovery ([#167](https://github.com/rararulab/rara/issues/167))
- **mita**: Add user_id parameter to read-tape tool ([#180](https://github.com/rararulab/rara/issues/180))
- **app**: Use correct user identity in E2E anchor checkout test ([#188](https://github.com/rararulab/rara/issues/188))
- **gateway**: Address code review feedback for telegram listener ([#199](https://github.com/rararulab/rara/issues/199))
- **gateway**: Remove /shutdown command to prevent unrecoverable state ([#199](https://github.com/rararulab/rara/issues/199))
- **gateway**: Add proxy + timeout support to gateway Telegram bot ([#205](https://github.com/rararulab/rara/issues/205))
- **mcp**: Use correct npm package name for context-mode ([#208](https://github.com/rararulab/rara/issues/208))
- **gateway**: Skip duplicate auto-update when detector publishes stale state
- **app**: Prevent context-mode interceptor from causing agent loop
- **app**: Use whitelist + 32KB threshold for context-mode interceptor
- **app**: Disable context-mode output interceptor
- **kernel**: Address review feedback — configurable rate limit, memory eviction, serde parse
- **kernel**: Address PR review — gc wiring, clock-testable rate limiter, strum parsing ([#223](https://github.com/rararulab/rara/issues/223))
- **context-mode**: Switch to default-on interceptor with exclusion list ([#230](https://github.com/rararulab/rara/issues/230))
- **app**: Lower context-mode intercept threshold to 8KB ([#236](https://github.com/rararulab/rara/issues/236))
- **telegram**: Harden tool-call XML stripping for streaming edge cases ([#314](https://github.com/rararulab/rara/issues/314))
- **keyring-store**: Enable tokio and crypto-rust features to satisfy secret-service v4 runtime requirement ([#332](https://github.com/rararulab/rara/issues/332)) ([#333](https://github.com/rararulab/rara/issues/333))
- **agents**: Add marketplace tool to rara agent manifest ([#347](https://github.com/rararulab/rara/issues/347))
- **telegram**: Pre-render trace HTML for instant callback response ([#343](https://github.com/rararulab/rara/issues/343))
- Resolve all clippy warnings across codebase ([#313](https://github.com/rararulab/rara/issues/313))
- **skills**: Universal marketplace install support ([#354](https://github.com/rararulab/rara/issues/354))
- **skills**: Include install_repo in marketplace tool description ([#704](https://github.com/rararulab/rara/issues/704)) ([#706](https://github.com/rararulab/rara/issues/706))
- **app**: Skip rtk rewrite for find commands with compound predicates ([#705](https://github.com/rararulab/rara/issues/705)) ([#707](https://github.com/rararulab/rara/issues/707))
- **tg**: Wire McpManager into KernelBotServiceClient ([#720](https://github.com/rararulab/rara/issues/720)) ([#721](https://github.com/rararulab/rara/issues/721))
- **app**: Context-mode interceptor whitelist + summary + system prompt ([#722](https://github.com/rararulab/rara/issues/722)) ([#732](https://github.com/rararulab/rara/issues/732))
- Remove screenshot ([#741](https://github.com/rararulab/rara/issues/741))
- **kernel**: Align OpenAI driver wire format with API spec ([#743](https://github.com/rararulab/rara/issues/743)) ([#745](https://github.com/rararulab/rara/issues/745))
- **app**: Bash tool — spawn with process group, incremental read, clean kill ([#744](https://github.com/rararulab/rara/issues/744)) ([#746](https://github.com/rararulab/rara/issues/746))
- **app**: Use actual MCP tool name in context-mode prompt fragment ([#751](https://github.com/rararulab/rara/issues/751)) ([#753](https://github.com/rararulab/rara/issues/753))
- **kernel**: Make interceptor prompt fragment dynamic to track MCP state ([#763](https://github.com/rararulab/rara/issues/763)) ([#769](https://github.com/rararulab/rara/issues/769))
- **kernel**: Tune agent loop timeouts for responsiveness ([#770](https://github.com/rararulab/rara/issues/770)) ([#772](https://github.com/rararulab/rara/issues/772))
- **app**: Correct ctx_search parameter format in context-mode prompt ([#799](https://github.com/rararulab/rara/issues/799)) ([#800](https://github.com/rararulab/rara/issues/800))
- **app**: Start wechat adapter polling loop on boot ([#848](https://github.com/rararulab/rara/issues/848)) ([#850](https://github.com/rararulab/rara/issues/850))
- **channels**: Prioritize filesystem credentials over settings for wechat adapter ([#867](https://github.com/rararulab/rara/issues/867)) ([#868](https://github.com/rararulab/rara/issues/868))
- **tools**: Sanitize discover-tools query to strip stray JSON characters ([#908](https://github.com/rararulab/rara/issues/908)) ([#909](https://github.com/rararulab/rara/issues/909))
- **tools**: Enrich marketplace tool parameter descriptions for LLM usability ([#911](https://github.com/rararulab/rara/issues/911)) ([#912](https://github.com/rararulab/rara/issues/912))
- **kernel**: Use parking_lot mutex ([#927](https://github.com/rararulab/rara/issues/927)) ([#933](https://github.com/rararulab/rara/issues/933))

### Documentation

- Add configuration guide, remove config.toml support
- Update README to reflect tape-based architecture ([#783](https://github.com/rararulab/rara/issues/783)) ([#784](https://github.com/rararulab/rara/issues/784))

### Features

- Full-stack integration — DB repos, REST API, Docker Compose, app wiring
- Notify & scheduler modules with background workers and API routes
- Add analytics API routes and Docker app service ([#18](https://github.com/rararulab/rara/issues/18))
- **notify**: Implement noop notification sender
- **notify**: Implement Telegram sender via teloxide
- **app**: Integrate JobSpyDriver into composition root
- **ai**: Integrate rig-core SDK, replace stubbed providers
- **telegram**: Create bot crate with message receiving
- Implement telegram bot worker and JD parse pipeline
- **job-source**: Add job discovery API endpoint and connect frontend
- **server**: Implement singleflight request dedup middleware ([#77](https://github.com/rararulab/rara/issues/77))
- **domain**: Add saved-job domain crate with CRUD API ([#81](https://github.com/rararulab/rara/issues/81))
- **workers**: Add saved job GC worker for expired links ([#83](https://github.com/rararulab/rara/issues/83))
- **domain**: Add saved job crawl + AI analysis pipeline ([#82](https://github.com/rararulab/rara/issues/82))
- Integrate saved jobs UI, MinIO config, and standalone telegram bot
- Add runtime settings with hot reload for ai and telegram
- **notifications**: Rebuild observability around pgmq queue semantics
- **agents**: Add proactive agent with personality (Agent Soul)
- **workers**: Implement agent scheduler and schedule tools ([#144](https://github.com/rararulab/rara/issues/144))
- **telegram**: Add group chat support with mention-based triggering
- **openapi**: Add OpenAPI support with utoipa + Swagger UI ([#159](https://github.com/rararulab/rara/issues/159))
- **pipeline**: Add cron scheduling for automatic pipeline runs ([#220](https://github.com/rararulab/rara/issues/220))
- **contacts**: Add telegram contacts allowlist ([#232](https://github.com/rararulab/rara/issues/232))
- **infisical**: Add Infisical secrets manager integration ([#267](https://github.com/rararulab/rara/issues/267))
- **config**: Replace Infisical with Consul KV as configuration source ([#288](https://github.com/rararulab/rara/issues/288))
- Load app config from consul kv via rs-consul
- **memory**: Mem0 on-demand K8s pod with LazyMem0Client ([#321](https://github.com/rararulab/rara/issues/321))
- **app**: Validate platform identity in ChatServiceBridge ([#363](https://github.com/rararulab/rara/issues/363))
- **app**: Wire real deps into I/O Bus pipeline ([#371](https://github.com/rararulab/rara/issues/371))
- **app**: Wire WebAdapter into I/O Bus pipeline and HTTP server
- **cli**: Add interactive chat command with TerminalAdapter ([#381](https://github.com/rararulab/rara/issues/381))
- **kernel**: Multi-provider LLM architecture with ProviderRegistry ([#421](https://github.com/rararulab/rara/issues/421))
- **user**: Backend auth — JWT + user domain crate ([#428](https://github.com/rararulab/rara/issues/428))
- **channels**: Add Telegram account linking via /link command ([#430](https://github.com/rararulab/rara/issues/430))
- **boot**: Channel binding-aware session resolver + misc fixes
- **kernel**: Inject per-process syscall tools into ToolRegistry ([#443](https://github.com/rararulab/rara/issues/443))
- **kernel**: Integrate agentfs-sdk for KV + ToolCall audit ([#451](https://github.com/rararulab/rara/issues/451))
- **kernel**: Settings-driven SandboxConfig + hot reload ([#453](https://github.com/rararulab/rara/issues/453))
- **agent**: Implement Mita background proactive agent ([#72](https://github.com/rararulab/rara/issues/72))
- **memory**: Add information writeback and tape compaction ([#73](https://github.com/rararulab/rara/issues/73))
- **kernel**: Wire knowledge layer into kernel event loop and boot sequence ([#81](https://github.com/rararulab/rara/issues/81))
- **tools**: Add SettingsTool for runtime config introspection ([#84](https://github.com/rararulab/rara/issues/84))
- **gateway**: Add supervisor foundation — spawn, health check, restart agent ([#92](https://github.com/rararulab/rara/issues/92))
- **gateway**: Add update detector — git fetch + rev comparison ([#93](https://github.com/rararulab/rara/issues/93))
- **config**: Support per-agent model override in config.yaml ([#99](https://github.com/rararulab/rara/issues/99))
- **gateway**: Wire update detector → executor → supervisor restart pipeline ([#102](https://github.com/rararulab/rara/issues/102))
- **gateway**: Send auto-update lifecycle notifications to Telegram channel ([#109](https://github.com/rararulab/rara/issues/109))
- **gateway**: Use sysinfo for rich system context in notifications ([#112](https://github.com/rararulab/rara/issues/112))
- **gateway**: Enable sccache for staging worktree builds
- **gateway**: Make commit rev a clickable GitHub link in notifications ([#114](https://github.com/rararulab/rara/issues/114))
- **symphony**: Add SymphonyService and integrate with rara-app ([#119](https://github.com/rararulab/rara/issues/119))
- **config**: Add unflatten_from_settings() for KV → config struct roundtrip ([#121](https://github.com/rararulab/rara/issues/121))
- **config**: Add ConfigFileSync for bidirectional config <-> settings sync ([#121](https://github.com/rararulab/rara/issues/121))
- **config**: Wire ConfigFileSync into startup, remove seed_defaults ([#121](https://github.com/rararulab/rara/issues/121))
- **kernel**: Dynamic MCP tool injection into agent loop ([#126](https://github.com/rararulab/rara/issues/126))
- **tools**: Resolve tool working directory to workspace_dir
- **kernel**: Usage collection, tape tools, and context contract ([#130](https://github.com/rararulab/rara/issues/130))
- **kernel**: KernelEvent::SendNotification + fix PublishEvent syscall ([#137](https://github.com/rararulab/rara/issues/137))
- **symphony**: Replace ralph sidecar with per-issue `ralph run` subprocess
- **kernel**: Harden tape-handoff prompt and elevate to core protocol ([#148](https://github.com/rararulab/rara/issues/148))
- Align chat tui with openfang
- **kernel**: Add desired_session_key to spawn_with_input ([#164](https://github.com/rararulab/rara/issues/164))
- **mita**: Add updated_since filter to list-sessions tool ([#169](https://github.com/rararulab/rara/issues/169))
- **memory**: User tape knowledge distillation via anchor ([#170](https://github.com/rararulab/rara/issues/170))
- **soul**: Implement evolve-soul tool and auto-notifications for Mita tools
- **tools**: Add set-avatar tool for Telegram bot profile photo ([#189](https://github.com/rararulab/rara/issues/189))
- **kernel+app**: Persist image metadata to session & add get-session-info tool ([#192](https://github.com/rararulab/rara/issues/192))
- **tui**: Integrate kernel CommandHandler into chat TUI ([#194](https://github.com/rararulab/rara/issues/194))
- **gateway**: Move bot_token + notification_channel_id into GatewayConfig ([#199](https://github.com/rararulab/rara/issues/199))
- **gateway**: Add GatewayTelegramListener with management commands ([#199](https://github.com/rararulab/rara/issues/199))
- **mcp**: Add InterceptorStats to ContextModeInterceptor ([#209](https://github.com/rararulab/rara/issues/209))
- **mcp**: Heartbeat reconnection restores context-mode interceptor ([#209](https://github.com/rararulab/rara/issues/209))
- **gateway**: Show build duration in auto-update notifications
- **channels**: Wire GroupPolicy into TelegramConfig
- **kernel**: Wire IngressRateLimiter into IOSubsystem resolve path
- **gateway**: Add ProcessMonitor with snapshot and threshold types ([#252](https://github.com/rararulab/rara/issues/252))
- **gateway**: Wire ProcessMonitor into bootstrap and status endpoint ([#252](https://github.com/rararulab/rara/issues/252))
- **gateway**: Add /threshold and /stats Telegram commands ([#252](https://github.com/rararulab/rara/issues/252))
- **gateway**: Persist alert thresholds to gateway-state.yaml ([#266](https://github.com/rararulab/rara/issues/266))
- **telegram**: Register all command handlers and add slash menu ([#331](https://github.com/rararulab/rara/issues/331))
- **kernel,telegram**: Rara_message_id end-to-end tracing and debug_trace tool ([#337](https://github.com/rararulab/rara/issues/337))
- **kernel**: Background agent spawning with proactive result delivery ([#340](https://github.com/rararulab/rara/issues/340))
- **kernel**: Context folding — auto-anchor with pressure-driven summarization ([#357](https://github.com/rararulab/rara/issues/357))
- **memory**: Add structured user profile template for distillation ([#402](https://github.com/rararulab/rara/issues/402)) ([#406](https://github.com/rararulab/rara/issues/406))
- **kernel,telegram**: Auto-generate session title & redesign /sessions UI ([#434](https://github.com/rararulab/rara/issues/434))
- **channels**: Add /status command with session info and scheduled jobs ([#450](https://github.com/rararulab/rara/issues/450)) ([#453](https://github.com/rararulab/rara/issues/453))
- **dock**: Generative UI canvas workbench ([#424](https://github.com/rararulab/rara/issues/424))
- **kernel**: Read-file adaptive paging based on context window ([#468](https://github.com/rararulab/rara/issues/468)) ([#471](https://github.com/rararulab/rara/issues/471))
- **kernel**: Add browser automation subsystem via Lightpanda + CDP ([#473](https://github.com/rararulab/rara/issues/473))
- **gateway**: Send Telegram notification on shutdown ([#490](https://github.com/rararulab/rara/issues/490)) ([#491](https://github.com/rararulab/rara/issues/491))
- **kernel**: Inject installed skills into agent system prompt ([#487](https://github.com/rararulab/rara/issues/487))
- **channels**: Add session delete buttons and relative time in /sessions ([#492](https://github.com/rararulab/rara/issues/492))
- **acp**: Add native acp client crate ([#504](https://github.com/rararulab/rara/issues/504))
- **kernel**: Add PathScopeGuard for file-access scope enforcement ([#579](https://github.com/rararulab/rara/issues/579)) ([#582](https://github.com/rararulab/rara/issues/582))
- **kernel**: Add LlmModelLister and LlmEmbedder extension traits ([#762](https://github.com/rararulab/rara/issues/762)) ([#766](https://github.com/rararulab/rara/issues/766))
- **kernel**: Deferred tool loading — reduce per-turn token overhead ([#756](https://github.com/rararulab/rara/issues/756)) ([#768](https://github.com/rararulab/rara/issues/768))
- **kernel**: Per-tool execution timeout granularity ([#778](https://github.com/rararulab/rara/issues/778)) ([#782](https://github.com/rararulab/rara/issues/782))
- **kernel**: Stream bash stdout in real-time during tool execution ([#777](https://github.com/rararulab/rara/issues/777)) ([#788](https://github.com/rararulab/rara/issues/788))
- **app**: Port kota file tools — in-process grep/find, delete-file, create-directory ([#808](https://github.com/rararulab/rara/issues/808)) ([#810](https://github.com/rararulab/rara/issues/810))
- **channels**: Add WeChat iLink Bot channel adapter ([#827](https://github.com/rararulab/rara/issues/827)) ([#830](https://github.com/rararulab/rara/issues/830))
- **kernel**: Discover-tools finds skills ([#833](https://github.com/rararulab/rara/issues/833)) ([#835](https://github.com/rararulab/rara/issues/835))
- **app**: Add system-paths tool to expose directory layout ([#838](https://github.com/rararulab/rara/issues/838)) ([#840](https://github.com/rararulab/rara/issues/840))
- **tools**: Include parameter summaries in discover-tools results ([#925](https://github.com/rararulab/rara/issues/925)) ([#926](https://github.com/rararulab/rara/issues/926))

### Miscellaneous Tasks

- Establish job backend baseline
- Refactor
- Format
- Rename to rara
- Format & some improvement & prompt markdown
- Change default HTTP port from 3000 to 25555
- Format
- Make lint pass across workspace
- Format
- Format
- Clean repo
- Format
- Clean
- **app**: Remove dead register_mcp_tools, fix stale doc comment ([#69](https://github.com/rararulab/rara/issues/69))
- Format
- Support composio config
- Format chat tui files
- Optimize supervisor
- **gateway**: Add sysinfo dependency ([#252](https://github.com/rararulab/rara/issues/252))
- Format
- Add missing AGENT.md files for all crates ([#535](https://github.com/rararulab/rara/issues/535)) ([#539](https://github.com/rararulab/rara/issues/539))
- **app**: Remove sync_from_file_populates_kv test ([#678](https://github.com/rararulab/rara/issues/678)) ([#680](https://github.com/rararulab/rara/issues/680))
- Disable lightpanda

### Refactor

- Rename rsketch crates to job
- **yunara-store**: Remove domain repos, models, and conversions ([#40](https://github.com/rararulab/rara/issues/40))
- Move API routes from server to domain crates
- **ai**: Replace generic AiTaskKind with concrete agent structs
- Extract AI crate from domain/, create workers crate
- Move TelegramService to shared crate, add notify-driven workers
- **telegram**: Extract bot from worker framework into standalone service
- Split SavedJobPipeline into independent crawl + analyze workers ([#92](https://github.com/rararulab/rara/issues/92))
- Decouple telegram bot with grpc/http boundary ([#94](https://github.com/rararulab/rara/issues/94))
- Move TelegramService into telegram-bot crate, add outbox pattern
- Remove notify domain crate, replace with lightweight shared notify client
- Realign domain boundaries and rename domain crates
- **notify**: Route observability through NotifyClient
- Unify runtime state into AppState with init() and routes()
- Unify job domain into single JobService
- Extract crawl4ai into common crate and remove unused downloader
- Migrate to reversible migrations, use paths crate for sessions dir, simplify store
- Unify process model, integrate telegram-bot into app
- **job**: Remove saved jobs feature ([#236](https://github.com/rararulab/rara/issues/236))
- **infisical**: Implement config AsyncSource instead of env var injection ([#269](https://github.com/rararulab/rara/issues/269))
- Move chroma config from settings to app config
- Remove duplicate worker memory config type
- **settings**: Move SettingsSvc + ollama from domain/shared to backend-admin ([#310](https://github.com/rararulab/rara/issues/310))
- **memory**: Integrate new MemoryManager into tools, orchestrator, and settings ([#313](https://github.com/rararulab/rara/issues/313))
- Remove mem0 on-demand pod mode
- **app**: Replace telegram-bot crate with TelegramAdapter ([#354](https://github.com/rararulab/rara/issues/354))
- **app**: Wire I/O Bus pipeline alongside ChatService ([#365](https://github.com/rararulab/rara/issues/365))
- Remove ChannelBridge, ChannelRouter, and ChatServiceBridge ([#366](https://github.com/rararulab/rara/issues/366))
- **kernel**: Extract shared spawn logic, consolidate noop defaults ([#368](https://github.com/rararulab/rara/issues/368))
- **kernel**: Redesign executor as OS process model ([#374](https://github.com/rararulab/rara/issues/374))
- **kernel**: Unify SessionRepository trait, delete SessionManager ([#378](https://github.com/rararulab/rara/issues/378))
- **kernel**: Separate kernel/agent concerns, unify spawn, use CancellationToken
- **kernel**: Kernel owns I/O subsystem, boot() -> Kernel ([#380](https://github.com/rararulab/rara/issues/380))
- Remove legacy proactive agent and agent scheduler
- **kernel**: Unify all interactions into KernelEvent ([#400](https://github.com/rararulab/rara/issues/400))
- Remove job pipeline module and related code
- **settings**: Unify runtime settings into flat KV store ([#401](https://github.com/rararulab/rara/issues/401))
- **agents**: Simplify agents crate — AgentRole to kernel, inline prompts, rara only ([#408](https://github.com/rararulab/rara/issues/408))
- **kernel**: ProcessTable redesign — tree index, AgentRegistry, 3-path routing ([#419](https://github.com/rararulab/rara/issues/419))
- **boot,backend**: Split AppState into RaraState + BackendState ([#438](https://github.com/rararulab/rara/issues/438))
- **kernel,config**: Rename SpawnTool → SyscallTool and harden Consul KV config
- **kernel**: Migrate external callers to KernelHandle, demote Kernel methods ([#24](https://github.com/rararulab/rara/issues/24))
- **kernel**: Remove async-openai and legacy LLM provider layer
- **kernel**: Consolidate queue/KV/LLM subsystems, remove rara-queue crate
- **kernel**: Dissolve defaults/ module into domain modules ([#36](https://github.com/rararulab/rara/issues/36))
- Remove PGMQ NotifyClient — replaced by kernel egress
- Replace Consul with YAML config, continue SQLite migration, add session resumption
- Remove rara-k8s crate and clean up unused infrastructure
- Remove telegram contacts subsystem
- **boot**: 简化 boot 层，集成 tape store 和 session index ([#44](https://github.com/rararulab/rara/issues/44))
- **llm**: Per-provider default_model and fallback_models ([#47](https://github.com/rararulab/rara/issues/47))
- Make it compile
- **kernel**: Remove SessionResolver, simplify ChannelBinding ([#63](https://github.com/rararulab/rara/issues/63))
- **app**: Merge rara-boot into rara-app ([#69](https://github.com/rararulab/rara/issues/69))
- **tool**: Remove unused ToolLayer/ToolSource, flatten tools directory ([#74](https://github.com/rararulab/rara/issues/74))
- **kernel**: Make knowledge layer a required component, not optional ([#81](https://github.com/rararulab/rara/issues/81))
- **kernel**: Remove enabled flag from KnowledgeConfig ([#81](https://github.com/rararulab/rara/issues/81))
- **app**: Load KnowledgeConfig from YAML config, not settings ([#81](https://github.com/rararulab/rara/issues/81))
- **app**: Align knowledge config with settings-first architecture
- **gateway**: Use teloxide::Bot instead of raw reqwest for TG notifier ([#109](https://github.com/rararulab/rara/issues/109))
- **gateway**: Require Telegram config and enrich notification templates ([#109](https://github.com/rararulab/rara/issues/109))
- **app**: Require mita config and use humantime-serde for durations
- **config**: Add Serialize derive to all AppConfig types ([#121](https://github.com/rararulab/rara/issues/121))
- **app,backend-admin**: Adapt to simplified symphony — remove SymphonyStatusHandle
- **mita**: Make Mita a long-lived session with fixed tape ([#164](https://github.com/rararulab/rara/issues/164))
- **mita**: Replace submit_message with typed MitaDirective ([#171](https://github.com/rararulab/rara/issues/171))
- **app**: Remove MitaHeartbeatWorker, use kernel scheduler ([#183](https://github.com/rararulab/rara/issues/183))
- **tools**: Set-avatar reads from images_dir instead of URL ([#190](https://github.com/rararulab/rara/issues/190))
- **gateway**: Simplify notification template, remove sysinfo dep ([#199](https://github.com/rararulab/rara/issues/199))
- **gateway**: Rename notification_channel_id to chat_id ([#199](https://github.com/rararulab/rara/issues/199))
- **kernel**: Make output_interceptor dynamically swappable ([#209](https://github.com/rararulab/rara/issues/209))
- **tools**: Split composio meta-tool into 4 focused tools ([#234](https://github.com/rararulab/rara/issues/234))
- **app**: Migrate app, dock, and knowledge tools to ToolDef derive macro ([#512](https://github.com/rararulab/rara/issues/512)) ([#519](https://github.com/rararulab/rara/issues/519))
- **browser**: Remove BrowserConfig from AppConfig ([#525](https://github.com/rararulab/rara/issues/525)) ([#526](https://github.com/rararulab/rara/issues/526))
- **tool**: Typed Output associated type for ToolExecute ([#524](https://github.com/rararulab/rara/issues/524)) ([#533](https://github.com/rararulab/rara/issues/533))
- **tool**: Migrate acp tools to ToolExecute macro ([#569](https://github.com/rararulab/rara/issues/569))
- **acp**: Remove builtin agents and capture stderr in spawn errors ([#571](https://github.com/rararulab/rara/issues/571)) ([#572](https://github.com/rararulab/rara/issues/572))
- **acp**: Address code review findings across rara-acp crate ([#672](https://github.com/rararulab/rara/issues/672)) ([#676](https://github.com/rararulab/rara/issues/676))
- **tools**: Token diet — aggressive tool tiering + new file tools + browser prompt ([#805](https://github.com/rararulab/rara/issues/805)) ([#806](https://github.com/rararulab/rara/issues/806))
- **kernel**: Drop output interceptor ([#809](https://github.com/rararulab/rara/issues/809)) ([#811](https://github.com/rararulab/rara/issues/811))
- **kernel**: Tool schema diet — split tape + compress descriptions ([#825](https://github.com/rararulab/rara/issues/825)) ([#826](https://github.com/rararulab/rara/issues/826))
- **kernel**: Prompt diet tool tiering ([#831](https://github.com/rararulab/rara/issues/831)) ([#832](https://github.com/rararulab/rara/issues/832))
- **app**: Remove swagger-ui support ([#904](https://github.com/rararulab/rara/issues/904))
- **tools**: Split monolithic marketplace tool into 6 independent tools ([#922](https://github.com/rararulab/rara/issues/922)) ([#923](https://github.com/rararulab/rara/issues/923))

### Testing

- **config**: Add integration test for ConfigFileSync file→KV sync ([#121](https://github.com/rararulab/rara/issues/121))
- **app**: Add E2E test for anchor checkout conversation flow ([#198](https://github.com/rararulab/rara/issues/198))

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

- Add version to internal workspace dependencies for crates.io publishing

### Features

- Integrate Buf for multi-language gRPC code generation
- Add worker abstraction and refactor common crate structure
- **worker**: Add fallible workers and pause modes

### Miscellaneous Tasks

- Update project structure and dependencies
- Apply clippy lints and fix warnings across workspace
- Make clippy happy
- Release v0.0.11

### Refactor

- Update package paths and service names for HelloService

<!-- generated by git-cliff -->

## [0.0.11] - 2026-01-20

### Features

- Integrate Buf for multi-language gRPC code generation
- Add worker abstraction and refactor common crate structure
- **worker**: Add fallible workers and pause modes

### Miscellaneous Tasks

- Update project structure and dependencies
- Apply clippy lints and fix warnings across workspace
- Make clippy happy

### Refactor

- Update package paths and service names for HelloService

<!-- generated by git-cliff -->
