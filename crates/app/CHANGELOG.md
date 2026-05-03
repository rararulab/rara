# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.0.1] - 2026-05-03

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
- **agents**: Mita_system_prompt string formatting and minor cleanups ([#995](https://github.com/rararulab/rara/issues/995)) ([#996](https://github.com/rararulab/rara/issues/996))
- **kernel**: Bypass proxy for local/private-network LLM providers ([#1020](https://github.com/rararulab/rara/issues/1020)) ([#1021](https://github.com/rararulab/rara/issues/1021))
- Resolve contract violations found by code quality scan ([#1026](https://github.com/rararulab/rara/issues/1026)) ([#1030](https://github.com/rararulab/rara/issues/1030))
- **app**: Coerce bash timeout from string/humantime to Duration ([#1114](https://github.com/rararulab/rara/issues/1114)) ([#1115](https://github.com/rararulab/rara/issues/1115))
- **app**: Set browser User-Agent on http-fetch tool ([#1189](https://github.com/rararulab/rara/issues/1189)) ([#1190](https://github.com/rararulab/rara/issues/1190))
- **tests**: Stabilize e2e rara_paths init for CI ([#1199](https://github.com/rararulab/rara/issues/1199))
- **app**: Replace npx/python web server with bun run dev ([#1272](https://github.com/rararulab/rara/issues/1272)) ([#1273](https://github.com/rararulab/rara/issues/1273))
- **app**: Accept Duration-style map for bash timeout parameter ([#1307](https://github.com/rararulab/rara/issues/1307)) ([#1308](https://github.com/rararulab/rara/issues/1308))
- **tests**: Replace sleep with state polling in llm_error e2e test ([#1336](https://github.com/rararulab/rara/issues/1336)) ([#1337](https://github.com/rararulab/rara/issues/1337))
- **kernel**: Add deferred tier to task and spawn-background tools ([#1375](https://github.com/rararulab/rara/issues/1375)) ([#1377](https://github.com/rararulab/rara/issues/1377))
- **kernel**: Add feed dispatch task, SqliteFeedStore, and sync registry with admin routes ([#1419](https://github.com/rararulab/rara/issues/1419))
- **channels**: Route ask-user question back to originating Telegram topic ([#1461](https://github.com/rararulab/rara/issues/1461)) ([#1462](https://github.com/rararulab/rara/issues/1462))
- **channels**: Harden ask-user — identity gate, sensitive DM routing, inline options ([#1464](https://github.com/rararulab/rara/issues/1464)) ([#1465](https://github.com/rararulab/rara/issues/1465))
- **channels**: Identity gate compares kernel UserId, not platform id ([#1533](https://github.com/rararulab/rara/issues/1533)) ([#1536](https://github.com/rararulab/rara/issues/1536))
- **web**: Align detail + add exec trace ([#1608](https://github.com/rararulab/rara/issues/1608)) ([#1611](https://github.com/rararulab/rara/issues/1611))
- **deps**: Pin zlob to 1.3.2 for zig 0.16 compatibility ([#1665](https://github.com/rararulab/rara/issues/1665)) ([#1666](https://github.com/rararulab/rara/issues/1666))
- **ci**: Fix rust 1.95 clippy lints ([#1667](https://github.com/rararulab/rara/issues/1667)) ([#1668](https://github.com/rararulab/rara/issues/1668))
- **data-feeds**: Persist runtime status + last_error to DB ([#1705](https://github.com/rararulab/rara/issues/1705)) ([#1714](https://github.com/rararulab/rara/issues/1714))
- **web**: Forward send-file attachments to browser via stream event ([#1731](https://github.com/rararulab/rara/issues/1731)) ([#1741](https://github.com/rararulab/rara/issues/1741))
- **backend-admin**: Add CORS layer to admin HTTP surface ([#1734](https://github.com/rararulab/rara/issues/1734)) ([#1753](https://github.com/rararulab/rara/issues/1753))
- **channels**: Derive web user_id from authenticated owner, ignore client input ([#1763](https://github.com/rararulab/rara/issues/1763)) ([#1771](https://github.com/rararulab/rara/issues/1771))
- **app**: Implicit web platform mapping for owner ([#1779](https://github.com/rararulab/rara/issues/1779)) ([#1783](https://github.com/rararulab/rara/issues/1783))
- **server**: Apply CORS layer to all public routes including health ([#1832](https://github.com/rararulab/rara/issues/1832)) ([#1834](https://github.com/rararulab/rara/issues/1834))
- **web**: Make reply buffer always-on, remove YAML config knob ([#1831](https://github.com/rararulab/rara/issues/1831)) ([#1835](https://github.com/rararulab/rara/issues/1835))
- **store,kernel**: Root-cause SQLite writer contention causing silent trace loss ([#1843](https://github.com/rararulab/rara/issues/1843)) ([#1845](https://github.com/rararulab/rara/issues/1845))
- **channels**: Buffer per-session events when no WS receivers attached and replay on reattach ([#1882](https://github.com/rararulab/rara/issues/1882)) ([#1887](https://github.com/rararulab/rara/issues/1887))
- **web**: Inline reply buffer caps as const, revert #1882 config regression ([#1907](https://github.com/rararulab/rara/issues/1907)) ([#1908](https://github.com/rararulab/rara/issues/1908))
- **app**: Resolve ConfigFileSync path via XDG fallback ([#1981](https://github.com/rararulab/rara/issues/1981)) ([#1985](https://github.com/rararulab/rara/issues/1985))
- **backend-admin**: Route chat-model lister and embedder through DriverRegistry ([#2014](https://github.com/rararulab/rara/issues/2014)) ([#2017](https://github.com/rararulab/rara/issues/2017))

### Documentation

- Add configuration guide, remove config.toml support
- Update README to reflect tape-based architecture ([#783](https://github.com/rararulab/rara/issues/783)) ([#784](https://github.com/rararulab/rara/issues/784))
- Simplify readme with logo ([#1088](https://github.com/rararulab/rara/issues/1088)) ([#1089](https://github.com/rararulab/rara/issues/1089))
- Add inspired-by credits ([#1091](https://github.com/rararulab/rara/issues/1091)) ([#1092](https://github.com/rararulab/rara/issues/1092))

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
- **kernel**: Add ToolHint, UserQuestionManager, and ask-user tool ([#945](https://github.com/rararulab/rara/issues/945)) ([#952](https://github.com/rararulab/rara/issues/952))
- **kernel**: Codex OAuth as first-class LLM provider ([#950](https://github.com/rararulab/rara/issues/950)) ([#953](https://github.com/rararulab/rara/issues/953))
- **cmd**: TUI-Telegram feature parity ([#961](https://github.com/rararulab/rara/issues/961)) ([#984](https://github.com/rararulab/rara/issues/984))
- **agents**: Mita self-improving skill discovery loop ([#992](https://github.com/rararulab/rara/issues/992)) ([#993](https://github.com/rararulab/rara/issues/993))
- **kernel**: Telegram voice message STT via local whisper-server ([#998](https://github.com/rararulab/rara/issues/998)) ([#1003](https://github.com/rararulab/rara/issues/1003))
- **app**: Auto-manage whisper-server lifecycle ([#1081](https://github.com/rararulab/rara/issues/1081)) ([#1082](https://github.com/rararulab/rara/issues/1082))
- **web**: Voice message input via microphone recording ([#1084](https://github.com/rararulab/rara/issues/1084)) ([#1085](https://github.com/rararulab/rara/issues/1085))
- **browser**: Seamless Lightpanda integration via config.yaml ([#1109](https://github.com/rararulab/rara/issues/1109)) ([#1110](https://github.com/rararulab/rara/issues/1110))
- **kernel**: Batch file reads and 429 rate-limit recovery ([#1118](https://github.com/rararulab/rara/issues/1118)) ([#1119](https://github.com/rararulab/rara/issues/1119))
- **channels**: Add /debug command for Telegram message context retrieval ([#1127](https://github.com/rararulab/rara/issues/1127)) ([#1130](https://github.com/rararulab/rara/issues/1130))
- **channels**: Telegram voice reply via TTS ([#1163](https://github.com/rararulab/rara/issues/1163)) ([#1171](https://github.com/rararulab/rara/issues/1171))
- **kernel**: Add safety axes + concurrency partitioning ([#1186](https://github.com/rararulab/rara/issues/1186)) ([#1192](https://github.com/rararulab/rara/issues/1192))
- **app**: Generalize send-image into send-file for arbitrary file delivery ([#1213](https://github.com/rararulab/rara/issues/1213)) ([#1214](https://github.com/rararulab/rara/issues/1214))
- **kernel**: Add CodexDriver for ChatGPT backend Responses API ([#1246](https://github.com/rararulab/rara/issues/1246)) ([#1247](https://github.com/rararulab/rara/issues/1247))
- **app**: Auto-start web frontend server alongside Gateway ([#1266](https://github.com/rararulab/rara/issues/1266)) ([#1268](https://github.com/rararulab/rara/issues/1268))
- **app**: Integrate fff-search as native frecency-aware find/grep tools ([#1282](https://github.com/rararulab/rara/issues/1282)) ([#1283](https://github.com/rararulab/rara/issues/1283))
- **kernel**: Implement SkillNudge + MemoryNudge lifecycle hooks ([#1290](https://github.com/rararulab/rara/issues/1290)) ([#1292](https://github.com/rararulab/rara/issues/1292))
- **agents**: Make safety fragment act-by-default to improve proactivity ([#1320](https://github.com/rararulab/rara/issues/1320)) ([#1324](https://github.com/rararulab/rara/issues/1324))
- **kernel**: Runtime ack detection to prevent lazy LLM responses ([#1329](https://github.com/rararulab/rara/issues/1329)) ([#1330](https://github.com/rararulab/rara/issues/1330))
- **kernel**: Hermes-aligned agent loop efficiency improvements ([#1384](https://github.com/rararulab/rara/issues/1384)) ([#1387](https://github.com/rararulab/rara/issues/1387))
- **channels**: Show line-change stats in edit-file TG progress ([#1404](https://github.com/rararulab/rara/issues/1404)) ([#1408](https://github.com/rararulab/rara/issues/1408))
- **kernel**: Add SQLite FTS5 index for tape-search ([#1399](https://github.com/rararulab/rara/issues/1399)) ([#1414](https://github.com/rararulab/rara/issues/1414))
- **channels**: Add pinned session card for Telegram ([#1415](https://github.com/rararulab/rara/issues/1415)) ([#1417](https://github.com/rararulab/rara/issues/1417))
- **kimi-oauth**: Add Kimi Code OAuth provider via kimi-cli token sharing ([#1420](https://github.com/rararulab/rara/issues/1420)) ([#1423](https://github.com/rararulab/rara/issues/1423))
- **kernel**: Route FeedEvents to subscribers via SubscriptionRegistry ([#1429](https://github.com/rararulab/rara/issues/1429)) ([#1431](https://github.com/rararulab/rara/issues/1431))
- **channels**: Telegram forum topics — auto-create, route, and delete ([#1430](https://github.com/rararulab/rara/issues/1430)) ([#1440](https://github.com/rararulab/rara/issues/1440))
- **channels**: Forum topic naming — deep-link, LLM title sync, /rename ([#1460](https://github.com/rararulab/rara/issues/1460)) ([#1463](https://github.com/rararulab/rara/issues/1463))
- **llm**: Openrouter vision catalog ([#1480](https://github.com/rararulab/rara/issues/1480)) ([#1482](https://github.com/rararulab/rara/issues/1482))
- **channels**: /model 弹 inline keyboard 列表 ([#1575](https://github.com/rararulab/rara/issues/1575)) ([#1576](https://github.com/rararulab/rara/issues/1576))
- **scheduler**: Rebuild admin UI on kernel JobWheel ([#1686](https://github.com/rararulab/rara/issues/1686)) ([#1695](https://github.com/rararulab/rara/issues/1695))
- **backend-admin**: Bearer auth on admin HTTP surface with Principal extractor ([#1710](https://github.com/rararulab/rara/issues/1710)) ([#1721](https://github.com/rararulab/rara/issues/1721))
- **kernel,web**: Route background task replies back to originating channel ([#1793](https://github.com/rararulab/rara/issues/1793)) ([#1823](https://github.com/rararulab/rara/issues/1823))
- **telemetry**: Export OTLP traces to self-hosted Langfuse ([#1855](https://github.com/rararulab/rara/issues/1855)) ([#1859](https://github.com/rararulab/rara/issues/1859))
- **tool**: Expose run_code via boxlite ([#1700](https://github.com/rararulab/rara/issues/1700)) ([#1861](https://github.com/rararulab/rara/issues/1861))
- **telemetry**: Integrate Pyroscope continuous profiling ([#1857](https://github.com/rararulab/rara/issues/1857)) ([#1862](https://github.com/rararulab/rara/issues/1862))
- **backend**: Regenerate session title endpoint ([#1884](https://github.com/rararulab/rara/issues/1884)) ([#1889](https://github.com/rararulab/rara/issues/1889))
- **telemetry**: Adopt OTel GenAI + OpenInference semconv on spans ([#1856](https://github.com/rararulab/rara/issues/1856)) ([#1863](https://github.com/rararulab/rara/issues/1863))
- **sandbox**: Replace path-scope guard with sandbox-enforced FS boundary ([#1936](https://github.com/rararulab/rara/issues/1936)) ([#1946](https://github.com/rararulab/rara/issues/1946))
- **telemetry**: Export OTLP logs to self-hosted Loki ([#1949](https://github.com/rararulab/rara/issues/1949)) ([#1952](https://github.com/rararulab/rara/issues/1952))
- **kernel,sessions**: SQLite-backed session index with tape-derived state ([#2025](https://github.com/rararulab/rara/issues/2025)) ([#2038](https://github.com/rararulab/rara/issues/2038))
- **kernel,sessions,web**: Per-session active/archived status with sidebar filter ([#2043](https://github.com/rararulab/rara/issues/2043)) ([#2058](https://github.com/rararulab/rara/issues/2058))

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
- Remove symphony orchestrator ([#1432](https://github.com/rararulab/rara/issues/1432)) ([#1437](https://github.com/rararulab/rara/issues/1437))
- **app,web**: Remove rara-dock subsystem ([#1895](https://github.com/rararulab/rara/issues/1895)) ([#1900](https://github.com/rararulab/rara/issues/1900))
- **app**: Remove rara-composio integration ([#1894](https://github.com/rararulab/rara/issues/1894)) ([#1899](https://github.com/rararulab/rara/issues/1899))
- **app,integrations**: Remove rara-pg-credential-store ([#1903](https://github.com/rararulab/rara/issues/1903)) ([#1905](https://github.com/rararulab/rara/issues/1905))
- **app**: Drop unused agentfs-sdk dependency ([#1891](https://github.com/rararulab/rara/issues/1891)) ([#1906](https://github.com/rararulab/rara/issues/1906))
- **config**: Make top-level Config safe-by-default and improve missing-field error ([#1913](https://github.com/rararulab/rara/issues/1913)) ([#1915](https://github.com/rararulab/rara/issues/1915))
- **tests**: Drop scripted-LLM e2e and wiremock-based tests ([#1930](https://github.com/rararulab/rara/issues/1930)) ([#1933](https://github.com/rararulab/rara/issues/1933))
- **telemetry**: Map spans to Langfuse-recognized attributes ([#2002](https://github.com/rararulab/rara/issues/2002)) ([#2004](https://github.com/rararulab/rara/issues/2004))

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
- Remove dead code identified by desloppify scan ([#1025](https://github.com/rararulab/rara/issues/1025)) ([#1029](https://github.com/rararulab/rara/issues/1029))
- **kernel**: Fix KernelHandle API inconsistencies ([#1027](https://github.com/rararulab/rara/issues/1027)) ([#1031](https://github.com/rararulab/rara/issues/1031))
- Fix naming conventions and convention drift ([#1032](https://github.com/rararulab/rara/issues/1032)) ([#1035](https://github.com/rararulab/rara/issues/1035))
- Replace magic strings with typed enums ([#1033](https://github.com/rararulab/rara/issues/1033)) ([#1036](https://github.com/rararulab/rara/issues/1036))
- **kernel**: Introduce ToolName newtype for type-safe tool identifiers ([#1123](https://github.com/rararulab/rara/issues/1123)) ([#1133](https://github.com/rararulab/rara/issues/1133))
- **kernel**: Type-state InboundMessage<Unresolved/Resolved> ([#1125](https://github.com/rararulab/rara/issues/1125)) ([#1134](https://github.com/rararulab/rara/issues/1134))
- **workspace**: Extract browser/stt from kernel into driver crates ([#1146](https://github.com/rararulab/rara/issues/1146)) ([#1154](https://github.com/rararulab/rara/issues/1154))
- **kernel**: Clean up io.rs (typestate, constructors, dead code, hot path) ([#1180](https://github.com/rararulab/rara/issues/1180)) ([#1184](https://github.com/rararulab/rara/issues/1184))
- **kernel**: Unified background-agent framework ([#1631](https://github.com/rararulab/rara/issues/1631)) ([#1650](https://github.com/rararulab/rara/issues/1650))
- **kernel**: Agent fallback chain ([#1670](https://github.com/rararulab/rara/issues/1670)) ([#1671](https://github.com/rararulab/rara/issues/1671))
- **model**: Rename feed_events table to data_feed_events ([#1720](https://github.com/rararulab/rara/issues/1720)) ([#1725](https://github.com/rararulab/rara/issues/1725))
- **kernel**: Clean up scheduler review nits ([#1729](https://github.com/rararulab/rara/issues/1729)) ([#1735](https://github.com/rararulab/rara/issues/1735))
- **data-feed**: Snafu errors + admin gating ([#1739](https://github.com/rararulab/rara/issues/1739)) ([#1782](https://github.com/rararulab/rara/issues/1782))
- **kernel**: Rename rara_message_id to rara_turn_id ([#1978](https://github.com/rararulab/rara/issues/1978)) ([#1991](https://github.com/rararulab/rara/issues/1991))

### Revert

- **kernel**: Back out today's proactivity changes
- Restore April 13 changes previously rolled back

### Styling

- **browser**: Fix rustfmt alignment in BrowserFetchResult ([#1112](https://github.com/rararulab/rara/issues/1112)) ([#1113](https://github.com/rararulab/rara/issues/1113))

### Testing

- **config**: Add integration test for ConfigFileSync file→KV sync ([#121](https://github.com/rararulab/rara/issues/121))
- **app**: Add E2E test for anchor checkout conversation flow ([#198](https://github.com/rararulab/rara/issues/198))
- **kernel**: Add ScriptedLlmDriver and CI-ready E2E test harness ([#1172](https://github.com/rararulab/rara/issues/1172)) ([#1175](https://github.com/rararulab/rara/issues/1175))
- **kernel**: Tool call round-trip E2E test with FakeTool ([#1177](https://github.com/rararulab/rara/issues/1177)) ([#1182](https://github.com/rararulab/rara/issues/1182))
- **kernel**: Failure mode E2E tests for LLM/tool/iteration errors ([#1179](https://github.com/rararulab/rara/issues/1179)) ([#1188](https://github.com/rararulab/rara/issues/1188))
- **app**: Remove redundant e2e_scripted tests ([#1890](https://github.com/rararulab/rara/issues/1890)) ([#1892](https://github.com/rararulab/rara/issues/1892))
- **app**: Drop real-LLM soak coupling, add scripted lane-2 coverage ([#2016](https://github.com/rararulab/rara/issues/2016)) ([#2024](https://github.com/rararulab/rara/issues/2024))

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
