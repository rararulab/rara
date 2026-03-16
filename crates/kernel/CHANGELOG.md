# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.0.1] - 2026-03-16

### Bug Fixes

- **egress**: Fallback to persistent platform identities for stateless channels ([#25](https://github.com/rararulab/rara/issues/25))
- **kernel**: Unify agent_turn error type from String to KernelError
- **kernel,channels**: Prevent duplicate telegram messages and stuck Running processes
- **kernel,boot**: Address user tape code review issues ([#75](https://github.com/rararulab/rara/issues/75))
- **kernel**: Wire spawn_child result_tx through Session lifecycle ([#76](https://github.com/rararulab/rara/issues/76))
- **kernel**: Cleanup spawn_child agents after first turn completion ([#76](https://github.com/rararulab/rara/issues/76))
- **kernel**: Catch panics in turn task and log actual panic message ([#88](https://github.com/rararulab/rara/issues/88))
- **kernel**: Store origin_endpoint in Session to prevent cross-channel reply leaks ([#96](https://github.com/rararulab/rara/issues/96))
- **memory**: Deduplicate user message in LLM context assembly ([#101](https://github.com/rararulab/rara/issues/101))
- **security**: Enforce tool permissions in agent loop
- **kernel**: User-friendly context window error message ([#129](https://github.com/rararulab/rara/issues/129))
- **tools**: Rename all tool names to match OpenAI ^[a-zA-Z0-9-]+$ pattern
- **kernel**: Schedule-add parameter validation for LLM compatibility ([#132](https://github.com/rararulab/rara/issues/132))
- **kernel,symphony**: Offload blocking I/O to spawn_blocking to prevent tokio starvation
- **kernel**: Scheduled task isolation — independent tape and silent delivery ([#140](https://github.com/rararulab/rara/issues/140))
- **kernel**: Build meaningful summary for auto-handoff anchor
- Tape memory
- **memory**: Improve tape search relevance
- **mita**: Persist MitaDirective as Event entry in session tape ([#173](https://github.com/rararulab/rara/issues/173))
- **kernel**: Move syscall job_wheel persist to spawn_blocking ([#184](https://github.com/rararulab/rara/issues/184))
- **kernel**: Prevent orphan tape in checkout and add rollback ([#188](https://github.com/rararulab/rara/issues/188))
- **channels**: Emit TextClear to fix tool progress notifications ([#207](https://github.com/rararulab/rara/issues/207))
- **kernel**: Emit ToolCallStart before argument parsing ([#207](https://github.com/rararulab/rara/issues/207))
- **kernel**: Normalize empty tool call arguments to valid JSON
- **kernel**: Log successful tool calls
- **kernel**: Log tool call arguments at start
- **kernel**: Include request args in tool error log
- **kernel**: Use info level for LLM request/response logs
- **kernel**: Ensure ToolCallStart is emitted before ToolCallArgumentsDelta
- **kernel**: Address review feedback — configurable rate limit, memory eviction, serde parse
- **kernel**: Address PR review — gc wiring, clock-testable rate limiter, strum parsing ([#223](https://github.com/rararulab/rara/issues/223))
- **kernel**: Preserve original message in NonRetryable/RetryableServer errors ([#227](https://github.com/rararulab/rara/issues/227))
- **kernel**: 强制执行 max_concurrency 和 child_semaphore 并发限制 (#242, #243)
- **kernel**: Add default_execution_mode to worker manifest
- **telegram**: Harden tool-call XML stripping for streaming edge cases ([#314](https://github.com/rararulab/rara/issues/314))
- **kernel**: Mark TurnTrace as failed and emit warning when max_iterations exhausted ([#319](https://github.com/rararulab/rara/issues/319)) ([#326](https://github.com/rararulab/rara/issues/326))
- **llm**: Add frequency_penalty to prevent repetition loops ([#317](https://github.com/rararulab/rara/issues/317)) ([#318](https://github.com/rararulab/rara/issues/318))
- **kernel**: Skip empty notifications instead of sending placeholder string ([#334](https://github.com/rararulab/rara/issues/334)) ([#336](https://github.com/rararulab/rara/issues/336))
- **agents**: Add marketplace tool to rara agent manifest ([#347](https://github.com/rararulab/rara/issues/347))
- **telegram**: Pre-render trace HTML for instant callback response ([#343](https://github.com/rararulab/rara/issues/343))
- **kernel**: Drop PublishEvent with missing/blank payload.message ([#350](https://github.com/rararulab/rara/issues/350))
- Resolve all clippy warnings across codebase ([#313](https://github.com/rararulab/rara/issues/313))
- **kernel**: Suppress duplicate error message on user interrupt ([#355](https://github.com/rararulab/rara/issues/355))

### Documentation

- **memory**: Add detailed what/how/why comments to tape memory module ([#64](https://github.com/rararulab/rara/issues/64))
- **kernel**: Add detailed comments to start_llm_turn explaining lifecycle phases
- **telegram**: Add implementation comments for anchor tree flows
- **kernel**: Enrich checkout action description in TapeTool ([#202](https://github.com/rararulab/rara/issues/202))
- **kernel**: Add AGENT.md guidelines for IngressRateLimiter and GroupPolicy
- **kernel**: Add 'why' reasoning to AGENT.md guidelines
- **kernel**: Add AGENT.md section for tape-driven message rebuild + context budget ([#229](https://github.com/rararulab/rara/issues/229))

### Features

- **kernel**: Implement KernelHandle as event-queue-based public API ([#23](https://github.com/rararulab/rara/issues/23))
- **session**: 添加 SessionIndex trait 和 FileSessionIndex 实现 ([#43](https://github.com/rararulab/rara/issues/43))
- **cmd**: Improve TUI session Gantt chart with metrics overlay and time axis
- **memory**: Add user tape for cross-session user memory ([#70](https://github.com/rararulab/rara/issues/70))
- **kernel**: Add group chat proactive reply with two-step LLM judgment ([#71](https://github.com/rararulab/rara/issues/71))
- **kernel**: Add KnowledgeConfig struct ([#81](https://github.com/rararulab/rara/issues/81))
- **kernel**: Add knowledge layer — items, categories, embedding, extractor, tool ([#81](https://github.com/rararulab/rara/issues/81))
- **kernel**: Wire knowledge layer into kernel event loop and boot sequence ([#81](https://github.com/rararulab/rara/issues/81))
- **channels**: Support sending images to users in Telegram ([#91](https://github.com/rararulab/rara/issues/91))
- **memory**: Add source_ids to compaction anchor and entry lookup by ID ([#104](https://github.com/rararulab/rara/issues/104))
- **memory**: Expose source references in knowledge search and add resolve_sources ([#105](https://github.com/rararulab/rara/issues/105))
- **memory**: Support fork from specific entry ID ([#107](https://github.com/rararulab/rara/issues/107))
- **agent**: Emit intent/progress during long multi-step tool executions ([#116](https://github.com/rararulab/rara/issues/116))
- **kernel**: Dynamic MCP tool injection into agent loop ([#126](https://github.com/rararulab/rara/issues/126))
- **kernel**: Replace oneshot result channel with mpsc AgentEvent channel ([#127](https://github.com/rararulab/rara/issues/127))
- **kernel**: Run_agent_loop emits milestones via mpsc channel ([#127](https://github.com/rararulab/rara/issues/127))
- **kernel**: Exec_spawn collects milestones into tool result ([#127](https://github.com/rararulab/rara/issues/127))
- **kernel**: Usage collection, tape tools, and context contract ([#130](https://github.com/rararulab/rara/issues/130))
- **llm**: Image compression pipeline for vision input ([#131](https://github.com/rararulab/rara/issues/131))
- **kernel**: Add scheduled task system ([#132](https://github.com/rararulab/rara/issues/132))
- **kernel**: Auto-handoff on context window overflow ([#134](https://github.com/rararulab/rara/issues/134))
- **kernel**: ScheduledJobAgent + enriched task notifications ([#135](https://github.com/rararulab/rara/issues/135))
- **kernel**: KernelEvent::SendNotification + fix PublishEvent syscall ([#137](https://github.com/rararulab/rara/issues/137))
- **kernel**: Runtime context guard with token feedback ([#149](https://github.com/rararulab/rara/issues/149))
- **llm**: Add Message::tool_result_multimodal() constructor
- **kernel**: Add ToolOutput type and update AgentTool::execute() signature
- **kernel**: Add desired_session_key to spawn_with_input ([#164](https://github.com/rararulab/rara/issues/164))
- **kernel**: Store LLM usage metadata on assistant tape entries ([#165](https://github.com/rararulab/rara/issues/165))
- **memory**: Add estimated_context_tokens to TapeInfo ([#165](https://github.com/rararulab/rara/issues/165))
- **kernel**: Expose estimated_context_tokens in tape.info tool ([#165](https://github.com/rararulab/rara/issues/165))
- **kernel**: Use estimated_context_tokens in context pressure warnings ([#165](https://github.com/rararulab/rara/issues/165))
- **memory**: User tape knowledge distillation via anchor ([#170](https://github.com/rararulab/rara/issues/170))
- **kernel**: Render soul prompt with runtime state via SoulRenderer ([#174](https://github.com/rararulab/rara/issues/174))
- **kernel**: Add mood inference hook at end of agent loop ([#176](https://github.com/rararulab/rara/issues/176))
- **kernel**: Add rate limit retry with exponential backoff for LLM calls
- **telegram**: Add /anchors and /checkout commands
- **kernel**: Add checkout action to tape tool ([#188](https://github.com/rararulab/rara/issues/188))
- **kernel**: Teach LLM about anchor navigation in runtime prompt ([#188](https://github.com/rararulab/rara/issues/188))
- **kernel**: Inject SessionIndex into TapeTool for real checkout ([#193](https://github.com/rararulab/rara/issues/193))
- **kernel**: Add checkout_root action to TapeTool ([#204](https://github.com/rararulab/rara/issues/204))
- **channels**: Include raw args in tool parse failure error ([#207](https://github.com/rararulab/rara/issues/207))
- **kernel**: Log LLM request and response at debug level
- **kernel**: Add GroupPolicy enum to channel types (#219-adjacent)
- **kernel**: Add IngressRateLimiter with sliding-window per-key limiting
- **kernel**: Wire IngressRateLimiter into IOSubsystem resolve path
- **kernel**: Add context budget for tool result truncation ([#228](https://github.com/rararulab/rara/issues/228))
- **kernel**: Add /msg_version command and session/manifest routing ([#257](https://github.com/rararulab/rara/issues/257))
- **channels**: Plan-execute TG 三级显示策略 + 单消息编辑流 ([#267](https://github.com/rararulab/rara/issues/267))
- **telegram**: Show input/output token counts in progress UX ([#304](https://github.com/rararulab/rara/issues/304))
- **kernel,telegram**: Rara_message_id end-to-end tracing and debug_trace tool ([#337](https://github.com/rararulab/rara/issues/337))
- **kernel**: Background agent spawning with proactive result delivery ([#340](https://github.com/rararulab/rara/issues/340))
- **kernel**: Context folding — auto-anchor with pressure-driven summarization ([#357](https://github.com/rararulab/rara/issues/357))

### Miscellaneous Tasks

- Establish job backend baseline
- Change default HTTP port from 3000 to 25555
- Format
- Tidy project
- Format code
- **kernel**: Replace From<(&str, Option<u16>)> with explicit classify_provider_error, add stage constants
- Rustfmt formatting pass, fix Helm replicas/workers from `true` to `1`
- Format
- Clean
- **kernel**: Add usearch dependency for knowledge layer ([#81](https://github.com/rararulab/rara/issues/81))
- Change jobs_path
- Format
- Add tool timeout
- Support composio config
- **kernel**: Add perf TODO for anchor tree session loading ([#188](https://github.com/rararulab/rara/issues/188))
- Format
- Format

### Refactor

- Decouple telegram bot with grpc/http boundary ([#94](https://github.com/rararulab/rara/issues/94))
- **kernel**: Extract SecuritySubsystem
- **kernel**: Extract AuditSubsystem
- **kernel**: Flatten KernelInner, add strum derives, instrument macros, and Arc type aliases
- **kernel**: Use join_all for concurrent event batch processing ([#20](https://github.com/rararulab/rara/issues/20))
- **kernel**: Migrate external callers to KernelHandle, demote Kernel methods ([#24](https://github.com/rararulab/rara/issues/24))
- **kernel**: Remove redundant spawn methods from Kernel
- **kernel**: Remove async-openai and legacy LLM provider layer
- **kernel**: Add OutboundEnvelope constructors, eliminate duplicate struct literals
- **kernel**: Extract routing helpers from handle_user_message
- **kernel**: Reduce ProcessHandle boilerplate with helper methods
- **kernel**: Consolidate queue/KV/LLM subsystems, remove rara-queue crate
- **kernel**: Replace manual map_err with snafu ResultExt ([#33](https://github.com/rararulab/rara/issues/33))
- **kernel**: Replace manual enum match with strum derives ([#34](https://github.com/rararulab/rara/issues/34))
- **kernel**: Replace manual Debug impls with derive_more::Debug ([#35](https://github.com/rararulab/rara/issues/35))
- **kernel**: Dissolve defaults/ module into domain modules ([#36](https://github.com/rararulab/rara/issues/36))
- **kernel**: Split session.rs into directory module, fix external import paths ([#36](https://github.com/rararulab/rara/issues/36))
- **kernel**: 将 RuntimeTable 从类型别名提升为领域结构体 ([#39](https://github.com/rararulab/rara/issues/39))
- **kernel**: 提取 DeliverySubsystem 子组件 ([#39](https://github.com/rararulab/rara/issues/39))
- **kernel**: 提取 SyscallDispatcher 子组件 ([#39](https://github.com/rararulab/rara/issues/39))
- **kernel**: 平铺过度拆分的子模块 ([#40](https://github.com/rararulab/rara/issues/40))
- Replace Consul with YAML config, continue SQLite migration, add session resumption
- **kernel**: Introduce EventBase for unified event metadata
- **llm**: Per-provider default_model and fallback_models ([#47](https://github.com/rararulab/rara/issues/47))
- **kernel**: Session-centric runtime ([#48](https://github.com/rararulab/rara/issues/48))
- **kernel**: Migrate from Memory+SessionRepository to tape ([#51](https://github.com/rararulab/rara/issues/51))
- Make it compile
- **kernel**: Remove SessionResolver, simplify ChannelBinding ([#63](https://github.com/rararulab/rara/issues/63))
- **kernel**: Extract TapeTool from SyscallTool into dedicated tool
- **kernel**: Improve TapeTool error handling, add between_anchors action ([#68](https://github.com/rararulab/rara/issues/68))
- **kernel**: Decouple proactive judgment into GroupMessage event ([#79](https://github.com/rararulab/rara/issues/79))
- **kernel**: Make knowledge layer a required component, not optional ([#81](https://github.com/rararulab/rara/issues/81))
- **kernel**: Remove enabled flag from KnowledgeConfig ([#81](https://github.com/rararulab/rara/issues/81))
- **app**: Align knowledge config with settings-first architecture
- **memory**: Compaction shrinks read set instead of deleting history ([#103](https://github.com/rararulab/rara/issues/103))
- **memory**: Define typed HandoffState contract for anchor state ([#106](https://github.com/rararulab/rara/issues/106))
- **kernel**: ScheduledTask as dedicated KernelEvent + notifications ([#133](https://github.com/rararulab/rara/issues/133))
- **kernel**: Split schedule-add into three LLM-friendly tools
- **kernel**: Rewrite tape tool description with topic-driven anchor and recall guidance
- **mita**: Replace submit_message with typed MitaDirective ([#171](https://github.com/rararulab/rara/issues/171))
- **soul**: Remove all fallback logic, use built-in defaults directly
- **kernel**: Spawn per event instead of join_all to prevent batch blocking ([#185](https://github.com/rararulab/rara/issues/185))
- **kernel**: Extract InMemorySessionIndex to shared test utility ([#188](https://github.com/rararulab/rara/issues/188))
- **kernel**: Move checkout_anchor to TapeService ([#188](https://github.com/rararulab/rara/issues/188))
- **kernel**: 每次迭代从 tape 重建 LLM messages，消除双写冗余 ([#229](https://github.com/rararulab/rara/issues/229))
- **kernel**: Replace Chinese prompts with English in agent loop ([#229](https://github.com/rararulab/rara/issues/229))
- **kernel**: Consolidate tool impls into tool/ module ([#264](https://github.com/rararulab/rara/issues/264))

### Testing

- **memory**: Add tests for estimated_context_tokens ([#165](https://github.com/rararulab/rara/issues/165))
- **kernel**: Add E2E tests for anchor checkout flow ([#197](https://github.com/rararulab/rara/issues/197))
- **kernel**: Add rate limiter window expiry test

<!-- generated by git-cliff -->
