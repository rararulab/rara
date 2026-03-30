# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.0.1] - 2026-03-30

### Bug Fixes

- Move contacts CRUD router back to backend-admin
- **backend-admin**: Resolve codex oauth route wiring for start/status/disconnect
- **codex-oauth**: Correct callback port and redirect URLs ([#324](https://github.com/rararulab/rara/issues/324))
- **codex-oauth**: Use OpenAI's pre-registered redirect URI and ephemeral callback server ([#324](https://github.com/rararulab/rara/issues/324))
- **codex-oauth**: Use static model list instead of API fetch ([#324](https://github.com/rararulab/rara/issues/324))
- **kernel**: Use ModelRepo for runtime model resolution, read Telegram token from settings
- **kernel**: Fix model type mismatch after Option<String> migration ([#421](https://github.com/rararulab/rara/issues/421))
- **kernel**: Add SessionKey::try_from_raw, remove Timer dead code, document kv panic safety
- Resolve all clippy warnings across codebase ([#313](https://github.com/rararulab/rara/issues/313))
- **kernel**: Openai wire format alignment ([#747](https://github.com/rararulab/rara/issues/747)) ([#750](https://github.com/rararulab/rara/issues/750))

### Documentation

- **codex**: Add integration architecture docs and inline comments

### Features

- **codex-oauth**: Fetch available models from OpenAI API ([#324](https://github.com/rararulab/rara/issues/324))
- **kernel**: Use HashMap for builtin agents and expose agent registry HTTP API
- **backend-admin**: Add kernel HTTP observability endpoints + RFC 9457 ProblemDetails ([#422](https://github.com/rararulab/rara/issues/422))
- **kernel**: Collect and expose per-turn agent traces ([#442](https://github.com/rararulab/rara/issues/442))
- **backend**: Add per-process WebSocket stream endpoint ([#447](https://github.com/rararulab/rara/issues/447))
- **config**: Wire ConfigFileSync into startup, remove seed_defaults ([#121](https://github.com/rararulab/rara/issues/121))
- **admin**: Add context-mode status endpoint ([#209](https://github.com/rararulab/rara/issues/209))
- **kernel**: Add /msg_version command and session/manifest routing ([#257](https://github.com/rararulab/rara/issues/257))
- **kernel**: Implement pause_turn circuit breaker for agent loop ([#506](https://github.com/rararulab/rara/issues/506)) ([#508](https://github.com/rararulab/rara/issues/508))
- **web**: Cascade viewer — agent execution trace side panel ([#513](https://github.com/rararulab/rara/issues/513))
- **kernel**: Add LlmModelLister and LlmEmbedder extension traits ([#762](https://github.com/rararulab/rara/issues/762)) ([#766](https://github.com/rararulab/rara/issues/766))
- **kernel**: Task tool — preset-based background agent delegation ([#845](https://github.com/rararulab/rara/issues/845)) ([#847](https://github.com/rararulab/rara/issues/847))
- **kernel**: Codex OAuth as first-class LLM provider ([#950](https://github.com/rararulab/rara/issues/950)) ([#953](https://github.com/rararulab/rara/issues/953))

### Miscellaneous Tasks

- Format
- Make lint pass across workspace
- Format
- Clean repo
- Clean
- Cleanup old memory + session code ([#45](https://github.com/rararulab/rara/issues/45))
- Format
- Format
- Format
- Remove unused llmfit-core dependency ([#348](https://github.com/rararulab/rara/issues/348))
- Add missing AGENT.md files for all crates ([#535](https://github.com/rararulab/rara/issues/535)) ([#539](https://github.com/rararulab/rara/issues/539))

### Performance

- **cascade**: Build CascadeTrace incrementally during agent loop ([#625](https://github.com/rararulab/rara/issues/625)) ([#632](https://github.com/rararulab/rara/issues/632))

### Refactor

- Consolidate 6 admin crates into rara-backend-admin ([#283](https://github.com/rararulab/rara/issues/283))
- **agent-core**: Move builtin prompts + FilePromptRepo from backend-admin ([#286](https://github.com/rararulab/rara/issues/286))
- **backend-admin**: Move pipeline and coding-task routers into backend-admin ([#291](https://github.com/rararulab/rara/issues/291))
- **backend-admin**: Move all domain routers into backend-admin ([#295](https://github.com/rararulab/rara/issues/295))
- **backend-admin**: Merge analytics, application, interview, scheduler domain crates ([#298](https://github.com/rararulab/rara/issues/298))
- **backend-admin**: Merge resume + job-pipeline into backend-admin ([#299](https://github.com/rararulab/rara/issues/299))
- Merge domain-job into backend-admin ([#303](https://github.com/rararulab/rara/issues/303))
- Remove notify admin routes from backend-admin ([#306](https://github.com/rararulab/rara/issues/306))
- Move contacts to telegram-bot, add ContactLookup trait ([#307](https://github.com/rararulab/rara/issues/307))
- **settings**: Move SettingsSvc + ollama from domain/shared to backend-admin ([#310](https://github.com/rararulab/rara/issues/310))
- **codex**: Move oauth core logic out of backend-admin
- **codex**: Move oauth core into integrations crate
- **codex**: Centralize oauth exchange and refresh in integration
- **kernel**: Move runner, context, subagent from agent-core to kernel ([#335](https://github.com/rararulab/rara/issues/335))
- Decompose rara-agents into kernel dispatcher + domain agents ([#337](https://github.com/rararulab/rara/issues/337))
- Remove legacy dispatcher from agents, admin backend, and frontend ([#343](https://github.com/rararulab/rara/issues/343))
- **app**: Replace telegram-bot crate with TelegramAdapter ([#354](https://github.com/rararulab/rara/issues/354))
- **chat**: Absorb rara-domain-chat into backend-admin ([#370](https://github.com/rararulab/rara/issues/370))
- Unify ChatMessage types, delete SessionRepoBridge ([#373](https://github.com/rararulab/rara/issues/373))
- Move AgentTool trait to kernel, tool impls to boot, delete tool-core ([#375](https://github.com/rararulab/rara/issues/375))
- **boot**: Remove domain tools, flatten tool module structure ([#377](https://github.com/rararulab/rara/issues/377))
- **chat**: Remove LLM execution from ChatService, delete ChatAgent
- Remove legacy proactive agent and agent scheduler
- Remove job pipeline module and related code
- **settings**: Unify runtime settings into flat KV store ([#401](https://github.com/rararulab/rara/issues/401))
- Remove legacy prompt management UI and API routes ([#413](https://github.com/rararulab/rara/issues/413))
- Remove legacy ai_tasks module and related dead code ([#418](https://github.com/rararulab/rara/issues/418))
- **boot,backend**: Split AppState into RaraState + BackendState ([#438](https://github.com/rararulab/rara/issues/438))
- **kernel**: Migrate external callers to KernelHandle, demote Kernel methods ([#24](https://github.com/rararulab/rara/issues/24))
- **kernel**: Remove async-openai and legacy LLM provider layer
- **kernel**: Consolidate queue/KV/LLM subsystems, remove rara-queue crate
- **kernel**: Split session.rs into directory module, fix external import paths ([#36](https://github.com/rararulab/rara/issues/36))
- **kernel**: 平铺过度拆分的子模块 ([#40](https://github.com/rararulab/rara/issues/40))
- Remove PGMQ NotifyClient — replaced by kernel egress
- Replace Consul with YAML config, continue SQLite migration, add session resumption
- Remove rara-k8s crate and clean up unused infrastructure
- Remove telegram contacts subsystem
- **boot**: 简化 boot 层，集成 tape store 和 session index ([#44](https://github.com/rararulab/rara/issues/44))
- **cmd,server**: Update to session-centric naming ([#50](https://github.com/rararulab/rara/issues/50))
- Make it compile
- **kernel**: Remove SessionResolver, simplify ChannelBinding ([#63](https://github.com/rararulab/rara/issues/63))
- **app**: Merge rara-boot into rara-app ([#69](https://github.com/rararulab/rara/issues/69))
- **app,backend-admin**: Adapt to simplified symphony — remove SymphonyStatusHandle
- **kernel**: Plan mode agent loop fixes (#648 #649 #650) ([#667](https://github.com/rararulab/rara/issues/667))
- **kernel**: Drop output interceptor ([#809](https://github.com/rararulab/rara/issues/809)) ([#811](https://github.com/rararulab/rara/issues/811))
- **kernel**: Fix KernelHandle API inconsistencies ([#1027](https://github.com/rararulab/rara/issues/1027)) ([#1031](https://github.com/rararulab/rara/issues/1031))

<!-- generated by git-cliff -->
