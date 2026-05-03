# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.0.1] - 2026-05-03

### Bug Fixes

- **agents**: Add snafu display attrs to Error variants for better diagnostics
- **agents**: Skip unparseable streaming chunks instead of aborting
- **agents**: Abort with clear message on incompatible streaming model
- **resume**: Rename migration to avoid timestamp conflict with pipeline_runs_events
- **agents**: Correct include_str! paths for prompt files ([#243](https://github.com/rararulab/rara/issues/243))
- **agents**: Restore compaction persistence via CompactionEffect ([#249](https://github.com/rararulab/rara/issues/249))
- **agent-core**: Configurable max_iterations with partial results on limit ([#260](https://github.com/rararulab/rara/issues/260))
- **tools**: Rename all tool names to match OpenAI ^[a-zA-Z0-9-]+$ pattern
- Tape memory
- **mita**: Add tape tool to Mita agent manifest ([#168](https://github.com/rararulab/rara/issues/168))
- **agents**: Prevent repetitive text output on simple greetings ([#201](https://github.com/rararulab/rara/issues/201))
- **agents**: 为 rara manifest 填充显式 tool allowlist ([#246](https://github.com/rararulab/rara/issues/246))
- **telegram**: Harden tool-call XML stripping for streaming edge cases ([#314](https://github.com/rararulab/rara/issues/314))
- **agents**: Add marketplace tool to rara agent manifest ([#347](https://github.com/rararulab/rara/issues/347))
- Resolve all clippy warnings across codebase ([#313](https://github.com/rararulab/rara/issues/313))
- **kernel**: Increase max_tokens to 2048 and optimize agent prompt ([#815](https://github.com/rararulab/rara/issues/815)) ([#816](https://github.com/rararulab/rara/issues/816))
- **agents**: Mita_system_prompt string formatting and minor cleanups ([#995](https://github.com/rararulab/rara/issues/995)) ([#996](https://github.com/rararulab/rara/issues/996))
- **agents**: Reinforce rara identity in system prompt ([#1024](https://github.com/rararulab/rara/issues/1024)) ([#1028](https://github.com/rararulab/rara/issues/1028))
- **agents**: Remove hardcoded max_iterations=25 from rara manifest ([#1325](https://github.com/rararulab/rara/issues/1325)) ([#1326](https://github.com/rararulab/rara/issues/1326))

### Documentation

- **streaming**: Add detailed comments to streaming implementation
- **agents**: Add comprehensive doc comments to subagent executor and tool
- **agents**: Add README explaining agent crate architecture ([#249](https://github.com/rararulab/rara/issues/249))
- Update README to reflect tape-based architecture ([#783](https://github.com/rararulab/rara/issues/783)) ([#784](https://github.com/rararulab/rara/issues/784))
- Simplify readme with logo ([#1088](https://github.com/rararulab/rara/issues/1088)) ([#1089](https://github.com/rararulab/rara/issues/1089))
- Add inspired-by credits ([#1091](https://github.com/rararulab/rara/issues/1091)) ([#1092](https://github.com/rararulab/rara/issues/1092))

### Features

- **agents**: Add agents crate with OpenRouter-based agent runner
- **sessions**: Add sessions crate and chat HTTP API ([#108](https://github.com/rararulab/rara/issues/108))
- **workers**: Integrate chat service into AppState
- **chat**: Integrate skills into agent loop — prompt injection + tool filtering ([#161](https://github.com/rararulab/rara/issues/161))
- **agents**: Model fallback chain ([#193](https://github.com/rararulab/rara/issues/193))
- Integrate composio
- **agents,chat**: Streaming agent runner + SSE endpoint ([#204](https://github.com/rararulab/rara/issues/204))
- **agents**: Replace openrouter-rs with async-openai + LlmProvider trait ([#206](https://github.com/rararulab/rara/issues/206))
- **agents**: Add AgentDefinition types and parser ([#242](https://github.com/rararulab/rara/issues/242))
- **agents**: Add subagent executor for single/chain/parallel ([#242](https://github.com/rararulab/rara/issues/242))
- **agents**: Add SubagentTool with single/chain/parallel modes ([#242](https://github.com/rararulab/rara/issues/242))
- **agents**: Add orchestrator module ([#243](https://github.com/rararulab/rara/issues/243))
- **agents**: Add builtin agent module with ChatAgent, ProactiveAgent, ScheduledAgent ([#249](https://github.com/rararulab/rara/issues/249))
- **agents**: Implement unified AgentDispatcher with LogStore, metrics, and REST API ([#270](https://github.com/rararulab/rara/issues/270))
- **agent-core**: Add ModelCapabilities detection and provider_hint plumbing
- **memory**: Add post-compaction recall and per-turn recall config ([#319](https://github.com/rararulab/rara/issues/319))
- **memory**: Recall strategy engine with agent-configurable rules ([#322](https://github.com/rararulab/rara/issues/322))
- **agent**: Implement Mita background proactive agent ([#72](https://github.com/rararulab/rara/issues/72))
- **memory**: Add information writeback and tape compaction ([#73](https://github.com/rararulab/rara/issues/73))
- **agent**: Emit intent/progress during long multi-step tool executions ([#116](https://github.com/rararulab/rara/issues/116))
- **kernel**: ScheduledJobAgent + enriched task notifications ([#135](https://github.com/rararulab/rara/issues/135))
- **kernel**: KernelEvent::SendNotification + fix PublishEvent syscall ([#137](https://github.com/rararulab/rara/issues/137))
- **memory**: User tape knowledge distillation via anchor ([#170](https://github.com/rararulab/rara/issues/170))
- **agents**: Integrate SoulLoader into agent manifest construction ([#172](https://github.com/rararulab/rara/issues/172))
- **mita**: Add soul evolution tools for background agent ([#177](https://github.com/rararulab/rara/issues/177))
- **soul**: Implement evolve-soul tool and auto-notifications for Mita tools
- **agents**: Add proactive behavior guidelines to rara and mita prompts
- **agents**: Improve rara interaction for heavy tasks ([#187](https://github.com/rararulab/rara/issues/187))
- **kernel**: Add /msg_version command and session/manifest routing ([#257](https://github.com/rararulab/rara/issues/257))
- **kernel**: Background agent spawning with proactive result delivery ([#340](https://github.com/rararulab/rara/issues/340))
- **memory**: Add note-taking strategy to Rara system prompt ([#403](https://github.com/rararulab/rara/issues/403)) ([#405](https://github.com/rararulab/rara/issues/405))
- **memory**: Add structured user profile template for distillation ([#402](https://github.com/rararulab/rara/issues/402)) ([#406](https://github.com/rararulab/rara/issues/406))
- **kernel,telegram**: Auto-generate session title & redesign /sessions UI ([#434](https://github.com/rararulab/rara/issues/434))
- **kernel**: External agent.md prompt ([#451](https://github.com/rararulab/rara/issues/451))
- **kernel**: Agent knowledge directory with index + on-demand loading ([#466](https://github.com/rararulab/rara/issues/466)) ([#469](https://github.com/rararulab/rara/issues/469))
- **kernel**: Add browser automation subsystem via Lightpanda + CDP ([#473](https://github.com/rararulab/rara/issues/473))
- **kernel**: Implement pause_turn circuit breaker for agent loop ([#506](https://github.com/rararulab/rara/issues/506)) ([#508](https://github.com/rararulab/rara/issues/508))
- **kernel**: Deferred tool loading — reduce per-turn token overhead ([#756](https://github.com/rararulab/rara/issues/756)) ([#768](https://github.com/rararulab/rara/issues/768))
- **kernel**: Task tool — preset-based background agent delegation ([#845](https://github.com/rararulab/rara/issues/845)) ([#847](https://github.com/rararulab/rara/issues/847))
- **kernel**: Add explore task preset and delegation routing ([#873](https://github.com/rararulab/rara/issues/873)) ([#875](https://github.com/rararulab/rara/issues/875))
- **kernel**: Inject deferred tool names into agent system prompt ([#902](https://github.com/rararulab/rara/issues/902)) ([#906](https://github.com/rararulab/rara/issues/906))
- **agents**: Mita self-improving skill discovery loop ([#992](https://github.com/rararulab/rara/issues/992)) ([#993](https://github.com/rararulab/rara/issues/993))
- **kernel**: Batch file reads and 429 rate-limit recovery ([#1118](https://github.com/rararulab/rara/issues/1118)) ([#1119](https://github.com/rararulab/rara/issues/1119))
- **kernel**: Self-continuation signal to prevent GPT stop-and-ask ([#1301](https://github.com/rararulab/rara/issues/1301)) ([#1306](https://github.com/rararulab/rara/issues/1306))
- **agents**: Make safety fragment act-by-default to improve proactivity ([#1320](https://github.com/rararulab/rara/issues/1320)) ([#1324](https://github.com/rararulab/rara/issues/1324))

### Miscellaneous Tasks

- Establish job backend baseline
- Rename to rara
- Format & some improvement & prompt markdown
- Change default HTTP port from 3000 to 25555
- Format
- Make lint pass across workspace
- Format
- Tidy project
- Format
- Format
- Format
- Add missing AGENT.md files for all crates ([#535](https://github.com/rararulab/rara/issues/535)) ([#539](https://github.com/rararulab/rara/issues/539))

### Refactor

- Decouple telegram bot with grpc/http boundary ([#94](https://github.com/rararulab/rara/issues/94))
- **agents**: Layered tool architecture (primitives + services)
- **agents**: Move generic primitive tools from workers to agents crate
- Add keyring-store crate, process group utils, layer READMEs, and dep upgrades
- Extract AgentTool to tool-core crate, McpManager derive Clone ([#198](https://github.com/rararulab/rara/issues/198))
- **agents**: Remove primitives, delegate to tool-core ([#199](https://github.com/rararulab/rara/issues/199))
- **agents**: Extract provider.rs + fix settings key reload
- **agents**: Restructure provider as directory module
- **agents**: Extract SubagentExecutor struct from free functions
- **agents**: Address code review findings ([#249](https://github.com/rararulab/rara/issues/249))
- **ai**: Remove rara-ai crate, move task agents into rara-agents ([#254](https://github.com/rararulab/rara/issues/254))
- **agents**: Extract dispatcher admin routes to rara-dispatcher-admin ([#272](https://github.com/rararulab/rara/issues/272))
- Migrate all prompt consumers to PromptRepo + cleanup legacy code ([#278](https://github.com/rararulab/rara/issues/278))
- Merge rara-prompt into agent-core + prompt-admin ([#280](https://github.com/rararulab/rara/issues/280))
- Remove compose_with_soul/resolve_soul and settings prompt fields ([#281](https://github.com/rararulab/rara/issues/281))
- **settings**: Move SettingsSvc + ollama from domain/shared to backend-admin ([#310](https://github.com/rararulab/rara/issues/310))
- **memory**: Integrate new MemoryManager into tools, orchestrator, and settings ([#313](https://github.com/rararulab/rara/issues/313))
- **memory**: Separate trigger timing for mem0, memos, and hindsight ([#318](https://github.com/rararulab/rara/issues/318))
- **agents**: Decompose AgentOrchestrator into AgentContext trait hierarchy ([#326](https://github.com/rararulab/rara/issues/326))
- Move memory-core into agent-core, add unified Memory trait, design kernel architecture
- **kernel**: Move runner, context, subagent from agent-core to kernel ([#335](https://github.com/rararulab/rara/issues/335))
- Remove legacy dispatcher from agents, admin backend, and frontend ([#343](https://github.com/rararulab/rara/issues/343))
- Delete orphaned rara-agents crate ([#344](https://github.com/rararulab/rara/issues/344))
- **kernel**: Consolidate queue/KV/LLM subsystems, remove rara-queue crate
- **kernel**: Introduce EventBase for unified event metadata
- **agents**: Optimize rara soul & system prompt for memory-first, anti-meta ([#95](https://github.com/rararulab/rara/issues/95))
- **agents**: Optimize nana prompt — stand-in positioning ([#98](https://github.com/rararulab/rara/issues/98))
- **agents**: Strengthen rara system prompt — identity, execution, transparency
- **app**: Require mita config and use humantime-serde for durations
- **soul**: Remove all fallback logic, use built-in defaults directly
- **agents**: Remove rara-soul dependency, soul resolved by kernel at runtime
- **soul**: Redesign rara personality to tsundere style
- **memory**: Remove MAX_USER_NOTES truncation, trust anchor boundary ([#407](https://github.com/rararulab/rara/issues/407))
- **kernel**: Plan mode agent loop fixes (#648 #649 #650) ([#667](https://github.com/rararulab/rara/issues/667))
- **kernel**: Prompt review — fix 12 findings ([#755](https://github.com/rararulab/rara/issues/755)) ([#758](https://github.com/rararulab/rara/issues/758))
- **kernel**: Flatten spawn-background params — remove nested manifest ([#764](https://github.com/rararulab/rara/issues/764)) ([#767](https://github.com/rararulab/rara/issues/767))
- **tools**: Token diet — aggressive tool tiering + new file tools + browser prompt ([#805](https://github.com/rararulab/rara/issues/805)) ([#806](https://github.com/rararulab/rara/issues/806))
- **agents**: Prompt diet — cut tokens ~49% ([#823](https://github.com/rararulab/rara/issues/823)) ([#824](https://github.com/rararulab/rara/issues/824))
- **agents**: Align system prompts with Claude Code patterns ([#841](https://github.com/rararulab/rara/issues/841)) ([#844](https://github.com/rararulab/rara/issues/844))
- **agents**: Optimize agent prompts with prompt-refinery framework ([#1105](https://github.com/rararulab/rara/issues/1105)) ([#1106](https://github.com/rararulab/rara/issues/1106))
- **kernel**: Introduce ToolName newtype for type-safe tool identifiers ([#1123](https://github.com/rararulab/rara/issues/1123)) ([#1133](https://github.com/rararulab/rara/issues/1133))
- **agents**: Expand rara anti-slop prompt ([#1587](https://github.com/rararulab/rara/issues/1587)) ([#1588](https://github.com/rararulab/rara/issues/1588))
- **kernel**: Unified background-agent framework ([#1631](https://github.com/rararulab/rara/issues/1631)) ([#1650](https://github.com/rararulab/rara/issues/1650))

### Revert

- **kernel**: Back out today's proactivity changes
- **kernel**: Back out remaining afternoon changes
- Restore April 13 changes previously rolled back

<!-- generated by git-cliff -->
