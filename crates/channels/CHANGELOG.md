# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.0.1] - 2026-03-20

### Bug Fixes

- **kernel**: Add SessionKey::try_from_raw, remove Timer dead code, document kv panic safety
- **channels**: Resolve RefMut-across-await, add Reply fallback, extend cleanup timeout ([#38](https://github.com/rararulab/rara/issues/38))
- **kernel,channels**: Prevent duplicate telegram messages and stuck Running processes
- Avoid duplicating telegram long replies after stream split
- Axum route
- **channels**: Add tool progress reporting in Telegram stream ([#87](https://github.com/rararulab/rara/issues/87))
- **telegram**: Implement unified command dispatch ([#108](https://github.com/rararulab/rara/issues/108))
- **tools**: Rename all tool names to match OpenAI ^[a-zA-Z0-9-]+$ pattern
- Tape memory
- Revert temporary work
- **telegram**: Handle photos without caption text ([#166](https://github.com/rararulab/rara/issues/166))
- **channels**: Refresh typing indicator directly in stream forwarder ([#182](https://github.com/rararulab/rara/issues/182))
- **gateway**: Add proxy + timeout support to gateway Telegram bot ([#205](https://github.com/rararulab/rara/issues/205))
- **channels**: Emit TextClear to fix tool progress notifications ([#207](https://github.com/rararulab/rara/issues/207))
- **guard**: Address PR review issues — auth, normalization, cleanup ([#220](https://github.com/rararulab/rara/issues/220))
- **tg**: Update progress timer even when no new events arrive ([#225](https://github.com/rararulab/rara/issues/225))
- **channels**: Show tool arguments in guard approval prompt
- **channels**: Strip tool call XML from TG text stream ([#276](https://github.com/rararulab/rara/issues/276))
- **telegram**: Harden tool-call XML stripping for streaming edge cases ([#314](https://github.com/rararulab/rara/issues/314))
- **telegram**: Pre-render trace HTML for instant callback response ([#343](https://github.com/rararulab/rara/issues/343))
- Resolve all clippy warnings across codebase ([#313](https://github.com/rararulab/rara/issues/313))
- **channels**: Render placeholder node for sessions with no anchors ([#436](https://github.com/rararulab/rara/issues/436)) ([#437](https://github.com/rararulab/rara/issues/437))
- **channels**: Remove stale message count from /sessions display ([#441](https://github.com/rararulab/rara/issues/441)) ([#442](https://github.com/rararulab/rara/issues/442))
- **channels**: Remove stale message count from session detail/switch display ([#448](https://github.com/rararulab/rara/issues/448)) ([#449](https://github.com/rararulab/rara/issues/449))
- **kernel**: Inject agent context into plan-mode planner ([#567](https://github.com/rararulab/rara/issues/567)) ([#576](https://github.com/rararulab/rara/issues/576))
- **tg**: Answer cascade callback immediately to prevent timeout ([#585](https://github.com/rararulab/rara/issues/585)) ([#587](https://github.com/rararulab/rara/issues/587))
- **tg**: Show tool call parameters in plan progress view ([#598](https://github.com/rararulab/rara/issues/598)) ([#599](https://github.com/rararulab/rara/issues/599))
- **guard**: Expand whitelist and fix approval timeout race (#604, #605) ([#609](https://github.com/rararulab/rara/issues/609))
- **tg**: Cascade button no response due to HTML truncation ([#691](https://github.com/rararulab/rara/issues/691)) ([#695](https://github.com/rararulab/rara/issues/695))
- **tg**: Show trace buttons for pure text replies ([#702](https://github.com/rararulab/rara/issues/702)) ([#703](https://github.com/rararulab/rara/issues/703))
- **tg**: Wire McpManager into KernelBotServiceClient ([#720](https://github.com/rararulab/rara/issues/720)) ([#721](https://github.com/rararulab/rara/issues/721))

### Features

- **channels**: Add StreamingMessage struct and StreamHub fields to TelegramAdapter ([#38](https://github.com/rararulab/rara/issues/38))
- **channels**: Implement spawn_stream_forwarder and flush_edit for TG streaming ([#38](https://github.com/rararulab/rara/issues/38))
- **channels**: Wire up TG stream forwarder in polling loop and egress reply ([#38](https://github.com/rararulab/rara/issues/38))
- **kernel**: Add group chat proactive reply with two-step LLM judgment ([#71](https://github.com/rararulab/rara/issues/71))
- **kernel,channels**: Expose Signal::Interrupt via SyscallTool, Telegram /stop, and Web API ([#80](https://github.com/rararulab/rara/issues/80))
- **channels**: Support sending images to users in Telegram ([#91](https://github.com/rararulab/rara/issues/91))
- **telegram**: Wire command handlers with KernelBotServiceClient ([#108](https://github.com/rararulab/rara/issues/108))
- **channels**: Enrich tool-call progress with argument summaries ([#115](https://github.com/rararulab/rara/issues/115))
- **telegram**: Compact tool-call progress display for long tasks ([#118](https://github.com/rararulab/rara/issues/118))
- **llm**: Image compression pipeline for vision input ([#131](https://github.com/rararulab/rara/issues/131))
- **channels**: Improve tool progress display detail ([#187](https://github.com/rararulab/rara/issues/187))
- **channels**: Add tool execution time and concurrency display ([#187](https://github.com/rararulab/rara/issues/187))
- **channels**: Show total turn elapsed time in progress display ([#187](https://github.com/rararulab/rara/issues/187))
- **channels**: Save uploaded photos to images_dir ([#191](https://github.com/rararulab/rara/issues/191))
- **tui**: Integrate kernel CommandHandler into chat TUI ([#194](https://github.com/rararulab/rara/issues/194))
- **telegram**: Show interceptor status in /mcp for context-mode ([#209](https://github.com/rararulab/rara/issues/209))
- **channels**: Show error reason in tool progress display ([#207](https://github.com/rararulab/rara/issues/207))
- **guard**: Integrate approval flow with Telegram inline keyboard ([#220](https://github.com/rararulab/rara/issues/220))
- **channels**: Render plan events in Telegram and Web adapters ([#251](https://github.com/rararulab/rara/issues/251))
- **channels**: Plan-execute TG 三级显示策略 + 单消息编辑流 ([#267](https://github.com/rararulab/rara/issues/267))
- **channels**: TG tool progress 语义化显示 ([#278](https://github.com/rararulab/rara/issues/278))
- **telegram**: Show input/output token counts in progress UX ([#304](https://github.com/rararulab/rara/issues/304))
- **telegram**: Register all command handlers and add slash menu ([#331](https://github.com/rararulab/rara/issues/331))
- **kernel,telegram**: Rara_message_id end-to-end tracing and debug_trace tool ([#337](https://github.com/rararulab/rara/issues/337))
- **kernel**: Background agent spawning with proactive result delivery ([#340](https://github.com/rararulab/rara/issues/340))
- **kernel,telegram**: Auto-generate session title & redesign /sessions UI ([#434](https://github.com/rararulab/rara/issues/434))
- **kernel**: Centralize loading hints with random selection ([#455](https://github.com/rararulab/rara/issues/455))
- **channels**: Add /status command with session info and scheduled jobs ([#450](https://github.com/rararulab/rara/issues/450)) ([#453](https://github.com/rararulab/rara/issues/453))
- **dock**: Generative UI canvas workbench ([#424](https://github.com/rararulab/rara/issues/424))
- **chat**: Support user image input in web and cli ([#475](https://github.com/rararulab/rara/issues/475))
- **channels**: Add session delete buttons and relative time in /sessions ([#492](https://github.com/rararulab/rara/issues/492))
- **kernel**: Implement pause_turn circuit breaker for agent loop ([#506](https://github.com/rararulab/rara/issues/506)) ([#508](https://github.com/rararulab/rara/issues/508))
- **web**: Cascade viewer — agent execution trace side panel ([#513](https://github.com/rararulab/rara/issues/513))
- **kernel**: Show LLM reasoning for tool calls in progress display ([#661](https://github.com/rararulab/rara/issues/661)) ([#664](https://github.com/rararulab/rara/issues/664))
- **channels**: Enhance guard approval UI with context, timing, and expiration ([#653](https://github.com/rararulab/rara/issues/653)) ([#665](https://github.com/rararulab/rara/issues/665))

### Miscellaneous Tasks

- Tidy project
- Format
- Clean
- Format
- Format
- Format
- **channels**: Upgrade reqwest 0.12 → 0.13 ([#283](https://github.com/rararulab/rara/issues/283)) ([#288](https://github.com/rararulab/rara/issues/288))
- Format
- Format
- Add missing AGENT.md files for all crates ([#535](https://github.com/rararulab/rara/issues/535)) ([#539](https://github.com/rararulab/rara/issues/539))

### Performance

- **cascade**: Build CascadeTrace incrementally during agent loop ([#625](https://github.com/rararulab/rara/issues/625)) ([#632](https://github.com/rararulab/rara/issues/632))

### Refactor

- **kernel**: Consolidate queue/KV/LLM subsystems, remove rara-queue crate
- Replace Consul with YAML config, continue SQLite migration, add session resumption
- Remove telegram contacts subsystem
- **kernel**: Introduce EventBase for unified event metadata
- **channels**: Adapt to session-centric kernel API ([#49](https://github.com/rararulab/rara/issues/49))
- **kernel**: Remove SessionResolver, simplify ChannelBinding ([#63](https://github.com/rararulab/rara/issues/63))
- **kernel**: Decouple proactive judgment into GroupMessage event ([#79](https://github.com/rararulab/rara/issues/79))
- **tools**: Split composio meta-tool into 4 focused tools ([#234](https://github.com/rararulab/rara/issues/234))
- **channels**: Use strum ToolKind enum for tool display mappings ([#278](https://github.com/rararulab/rara/issues/278))
- **tg**: Cascade viewer toggles in-place instead of sending new message ([#528](https://github.com/rararulab/rara/issues/528))
- **tg**: Unify plan and progress into single in-progress message ([#580](https://github.com/rararulab/rara/issues/580)) ([#583](https://github.com/rararulab/rara/issues/583))

### Styling

- **channels**: Fix rustfmt formatting ([#723](https://github.com/rararulab/rara/issues/723)) ([#728](https://github.com/rararulab/rara/issues/728))

### Testing

- **channels**: Add unit and integration tests for TG streaming ([#38](https://github.com/rararulab/rara/issues/38))
- **app**: Add E2E test for anchor checkout conversation flow ([#198](https://github.com/rararulab/rara/issues/198))

<!-- generated by git-cliff -->
