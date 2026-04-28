# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.0.1] - 2026-04-28

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
- **channels**: Adapt wechat login to iLink API v2 ([#839](https://github.com/rararulab/rara/issues/839)) ([#842](https://github.com/rararulab/rara/issues/842))
- **channels**: Correct wechat polling field names for msgs and user ID ([#869](https://github.com/rararulab/rara/issues/869)) ([#870](https://github.com/rararulab/rara/issues/870))
- **channels**: Handle iLink API type 1 text_item format in body_from_item_list ([#872](https://github.com/rararulab/rara/issues/872)) ([#874](https://github.com/rararulab/rara/issues/874))
- **channels**: Send wechat replies to bot account_id, not from_user_id ([#877](https://github.com/rararulab/rara/issues/877)) ([#879](https://github.com/rararulab/rara/issues/879))
- **channels**: Align sendmessage with iLink protocol per weclaw reference ([#884](https://github.com/rararulab/rara/issues/884)) ([#886](https://github.com/rararulab/rara/issues/886))
- **channels**: Implement correct iLink typing indicator protocol ([#887](https://github.com/rararulab/rara/issues/887)) ([#889](https://github.com/rararulab/rara/issues/889))
- **channels**: Handle Progress variant to trigger wechat typing indicator ([#891](https://github.com/rararulab/rara/issues/891)) ([#892](https://github.com/rararulab/rara/issues/892))
- **channels**: Always refresh progress timer once message exists ([#893](https://github.com/rararulab/rara/issues/893)) ([#895](https://github.com/rararulab/rara/issues/895))
- **channels**: Use kernel-resolved session key for web stream forwarder ([#1056](https://github.com/rararulab/rara/issues/1056)) ([#1057](https://github.com/rararulab/rara/issues/1057))
- **kernel**: Use SessionKey/ChannelType types in ChannelMessage and ChannelBinding ([#1120](https://github.com/rararulab/rara/issues/1120)) ([#1132](https://github.com/rararulab/rara/issues/1132))
- **kernel**: Explicit backpressure retry for IOError::Full in ingress pipeline ([#1148](https://github.com/rararulab/rara/issues/1148)) ([#1158](https://github.com/rararulab/rara/issues/1158))
- **drivers**: Structured STT errors with retry for transient failures ([#1164](https://github.com/rararulab/rara/issues/1164)) ([#1168](https://github.com/rararulab/rara/issues/1168))
- **channels**: Add missing ToolKind variants and show raw name for unknown tools ([#1191](https://github.com/rararulab/rara/issues/1191)) ([#1193](https://github.com/rararulab/rara/issues/1193))
- **tests**: Stabilize e2e rara_paths init for CI ([#1199](https://github.com/rararulab/rara/issues/1199))
- **telegram**: Avoid duplicate replies and render markdown tables ([#1203](https://github.com/rararulab/rara/issues/1203))
- **channels**: /debug shows all entry kinds via entries_by_message_id ([#1207](https://github.com/rararulab/rara/issues/1207)) ([#1209](https://github.com/rararulab/rara/issues/1209))
- **channels**: Panic on UTF-8 char boundary in markdown chunk_message ([#1258](https://github.com/rararulab/rara/issues/1258))
- **channels**: Route approval requests to originating chat, not primary_chat_id ([#1319](https://github.com/rararulab/rara/issues/1319)) ([#1322](https://github.com/rararulab/rara/issues/1322))
- **channels**: Guard streamed_visible_prefix behind actual TG message send ([#1334](https://github.com/rararulab/rara/issues/1334)) ([#1335](https://github.com/rararulab/rara/issues/1335))
- **channels**: Show file path in edit-file TG progress display ([#1401](https://github.com/rararulab/rara/issues/1401)) ([#1403](https://github.com/rararulab/rara/issues/1403))
- **kernel**: Session title stuck as Untitled after switching ([#1433](https://github.com/rararulab/rara/issues/1433)) ([#1436](https://github.com/rararulab/rara/issues/1436))
- **channels**: Route ask-user question back to originating Telegram topic ([#1461](https://github.com/rararulab/rara/issues/1461)) ([#1462](https://github.com/rararulab/rara/issues/1462))
- **channels**: Harden ask-user — identity gate, sensitive DM routing, inline options ([#1464](https://github.com/rararulab/rara/issues/1464)) ([#1465](https://github.com/rararulab/rara/issues/1465))
- **channels**: Route guard approval back to origin topic + identity gate ([#1466](https://github.com/rararulab/rara/issues/1466)) ([#1467](https://github.com/rararulab/rara/issues/1467))
- **channels**: Scope stale stream-state cleanup to its own epoch ([#1472](https://github.com/rararulab/rara/issues/1472)) ([#1478](https://github.com/rararulab/rara/issues/1478))
- **channels**: Wide table card layout ([#1483](https://github.com/rararulab/rara/issues/1483)) ([#1488](https://github.com/rararulab/rara/issues/1488))
- **channels**: Retry compact summary edit on 429 to preserve inline buttons ([#1484](https://github.com/rararulab/rara/issues/1484)) ([#1486](https://github.com/rararulab/rara/issues/1486))
- **channels**: Per-chat rate limiter + delta gating for TG edits ([#1510](https://github.com/rararulab/rara/issues/1510)) ([#1513](https://github.com/rararulab/rara/issues/1513))
- **channels**: Identity gate compares kernel UserId, not platform id ([#1533](https://github.com/rararulab/rara/issues/1533)) ([#1536](https://github.com/rararulab/rara/issues/1536))
- **web**: Surface agent errors via WebEvent ([#1573](https://github.com/rararulab/rara/issues/1573)) ([#1574](https://github.com/rararulab/rara/issues/1574))
- **ci**: Fix rust 1.95 clippy lints ([#1667](https://github.com/rararulab/rara/issues/1667)) ([#1668](https://github.com/rararulab/rara/issues/1668))
- **web**: Forward send-file attachments to browser via stream event ([#1731](https://github.com/rararulab/rara/issues/1731)) ([#1741](https://github.com/rararulab/rara/issues/1741))
- **channels**: Push live approval events to web UI ([#1745](https://github.com/rararulab/rara/issues/1745)) ([#1748](https://github.com/rararulab/rara/issues/1748))
- **channels**: Derive web user_id from authenticated owner, ignore client input ([#1763](https://github.com/rararulab/rara/issues/1763)) ([#1771](https://github.com/rararulab/rara/issues/1771))
- **web**: Make reply buffer always-on, remove YAML config knob ([#1831](https://github.com/rararulab/rara/issues/1831)) ([#1835](https://github.com/rararulab/rara/issues/1835))
- **kernel,channels**: Emit TextClear on laziness nudge to clear stale stream text ([#1852](https://github.com/rararulab/rara/issues/1852)) ([#1860](https://github.com/rararulab/rara/issues/1860))
- **web,channels**: Finish #1852 nits — TS StreamEvent variant + cross-crate doc xref ([#1869](https://github.com/rararulab/rara/issues/1869)) ([#1874](https://github.com/rararulab/rara/issues/1874))
- **channels**: Buffer per-session events when no WS receivers attached and replay on reattach ([#1882](https://github.com/rararulab/rara/issues/1882)) ([#1887](https://github.com/rararulab/rara/issues/1887))
- **web**: Inline reply buffer caps as const, revert #1882 config regression ([#1907](https://github.com/rararulab/rara/issues/1907)) ([#1908](https://github.com/rararulab/rara/issues/1908))
- **kernel,web**: Structured LLM error surfacing and suppress empty failure traces ([#1926](https://github.com/rararulab/rara/issues/1926)) ([#1938](https://github.com/rararulab/rara/issues/1938))
- **kernel,channels**: Demote three heartbeat log sources flooding Loki ([#1976](https://github.com/rararulab/rara/issues/1976)) ([#1983](https://github.com/rararulab/rara/issues/1983))

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
- **kernel**: Add tool call loop breaker ([#773](https://github.com/rararulab/rara/issues/773)) ([#775](https://github.com/rararulab/rara/issues/775))
- **kernel**: Stream bash stdout in real-time during tool execution ([#777](https://github.com/rararulab/rara/issues/777)) ([#788](https://github.com/rararulab/rara/issues/788))
- **channels**: Add WeChat iLink Bot channel adapter ([#827](https://github.com/rararulab/rara/issues/827)) ([#830](https://github.com/rararulab/rara/issues/830))
- **channels**: Log bot and user identity in wechat adapter ([#864](https://github.com/rararulab/rara/issues/864)) ([#865](https://github.com/rararulab/rara/issues/865))
- **channels**: Improve Telegram progress UX (#947, #948, #949) ([#951](https://github.com/rararulab/rara/issues/951))
- **kernel**: Add ToolHint, UserQuestionManager, and ask-user tool ([#945](https://github.com/rararulab/rara/issues/945)) ([#952](https://github.com/rararulab/rara/issues/952))
- **cmd**: TUI-Telegram feature parity ([#961](https://github.com/rararulab/rara/issues/961)) ([#984](https://github.com/rararulab/rara/issues/984))
- **kernel**: Telegram voice message STT via local whisper-server ([#998](https://github.com/rararulab/rara/issues/998)) ([#1003](https://github.com/rararulab/rara/issues/1003))
- **web**: Voice message input via microphone recording ([#1084](https://github.com/rararulab/rara/issues/1084)) ([#1085](https://github.com/rararulab/rara/issues/1085))
- **channels**: Convert LaTeX math to Unicode for Telegram display ([#1101](https://github.com/rararulab/rara/issues/1101)) ([#1102](https://github.com/rararulab/rara/issues/1102))
- **channels**: Add /debug command for Telegram message context retrieval ([#1127](https://github.com/rararulab/rara/issues/1127)) ([#1130](https://github.com/rararulab/rara/issues/1130))
- **cmd**: Add 'rara debug <message_id>' CLI command ([#1135](https://github.com/rararulab/rara/issues/1135)) ([#1136](https://github.com/rararulab/rara/issues/1136))
- **channels**: Telegram voice reply via TTS ([#1163](https://github.com/rararulab/rara/issues/1163)) ([#1171](https://github.com/rararulab/rara/issues/1171))
- **app**: Generalize send-image into send-file for arbitrary file delivery ([#1213](https://github.com/rararulab/rara/issues/1213)) ([#1214](https://github.com/rararulab/rara/issues/1214))
- **channels**: Port Claude Code spinner verbs for tool progress feedback ([#1220](https://github.com/rararulab/rara/issues/1220)) ([#1221](https://github.com/rararulab/rara/issues/1221))
- **channels**: Move spinner verb from phase line to footer ([#1295](https://github.com/rararulab/rara/issues/1295)) ([#1296](https://github.com/rararulab/rara/issues/1296))
- **channels**: Show tool name in progress phase line when summary is empty ([#1298](https://github.com/rararulab/rara/issues/1298)) ([#1299](https://github.com/rararulab/rara/issues/1299))
- **channels**: Hermes-style per-tool progress lines ([#1300](https://github.com/rararulab/rara/issues/1300))
- **channels**: Display subagent status in Telegram ([#1327](https://github.com/rararulab/rara/issues/1327)) ([#1328](https://github.com/rararulab/rara/issues/1328))
- **channels**: Add inline message Dashboard for Telegram ([#1389](https://github.com/rararulab/rara/issues/1389)) ([#1391](https://github.com/rararulab/rara/issues/1391))
- **channels**: Show line-change stats in edit-file TG progress ([#1404](https://github.com/rararulab/rara/issues/1404)) ([#1408](https://github.com/rararulab/rara/issues/1408))
- **channels**: Add pinned session card for Telegram ([#1415](https://github.com/rararulab/rara/issues/1415)) ([#1417](https://github.com/rararulab/rara/issues/1417))
- **channels**: Telegram forum topics — auto-create, route, and delete ([#1430](https://github.com/rararulab/rara/issues/1430)) ([#1440](https://github.com/rararulab/rara/issues/1440))
- **channels**: Telegram reply keyboard with session status + forum topic lifecycle ([#1454](https://github.com/rararulab/rara/issues/1454)) ([#1455](https://github.com/rararulab/rara/issues/1455))
- **channels**: Forum topic = independent session ([#1456](https://github.com/rararulab/rara/issues/1456)) ([#1457](https://github.com/rararulab/rara/issues/1457))
- **channels**: Forum topic naming — deep-link, LLM title sync, /rename ([#1460](https://github.com/rararulab/rara/issues/1460)) ([#1463](https://github.com/rararulab/rara/issues/1463))
- **channels**: Enhance Telegram pinned session card ([#1485](https://github.com/rararulab/rara/issues/1485)) ([#1493](https://github.com/rararulab/rara/issues/1493))
- **channels**: Wire reply keyboard into Telegram adapter ([#1490](https://github.com/rararulab/rara/issues/1490)) ([#1492](https://github.com/rararulab/rara/issues/1492))
- **channels**: Show model + context in TG pin floating preview ([#1541](https://github.com/rararulab/rara/issues/1541)) ([#1544](https://github.com/rararulab/rara/issues/1544))
- **channels**: /model 弹 inline keyboard 列表 ([#1575](https://github.com/rararulab/rara/issues/1575)) ([#1576](https://github.com/rararulab/rara/issues/1576))
- **llm**: MiniMax-M2 streaming robustness — driver + kernel + adapter ([#1630](https://github.com/rararulab/rara/issues/1630)) ([#1649](https://github.com/rararulab/rara/issues/1649))
- **backend-admin**: Bearer auth on admin HTTP surface with Principal extractor ([#1710](https://github.com/rararulab/rara/issues/1710)) ([#1721](https://github.com/rararulab/rara/issues/1721))
- **kernel,web**: Route background task replies back to originating channel ([#1793](https://github.com/rararulab/rara/issues/1793)) ([#1823](https://github.com/rararulab/rara/issues/1823))
- **kernel,channels,web**: Push session events for non-user tape mutations ([#1849](https://github.com/rararulab/rara/issues/1849)) ([#1858](https://github.com/rararulab/rara/issues/1858))
- **web,channels**: RaraAgent on a single per-session WS ([#1935](https://github.com/rararulab/rara/issues/1935)) ([#1947](https://github.com/rararulab/rara/issues/1947))
- **channels**: Add server-side WS keepalive ping ([#1967](https://github.com/rararulab/rara/issues/1967)) ([#1974](https://github.com/rararulab/rara/issues/1974))

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
- **channels**: Add debug logging to wechat polling loop ([#858](https://github.com/rararulab/rara/issues/858)) ([#859](https://github.com/rararulab/rara/issues/859))
- **channels**: Promote wechat adapter logs to info level ([#860](https://github.com/rararulab/rara/issues/860)) ([#861](https://github.com/rararulab/rara/issues/861))
- **channels**: Add delivery debug logging for wechat send path ([#881](https://github.com/rararulab/rara/issues/881)) ([#882](https://github.com/rararulab/rara/issues/882))
- **channels**: Log receiver_count in web broadcast_event ([#1516](https://github.com/rararulab/rara/issues/1516)) ([#1521](https://github.com/rararulab/rara/issues/1521))
- **app,web**: Remove rara-dock subsystem ([#1895](https://github.com/rararulab/rara/issues/1895)) ([#1900](https://github.com/rararulab/rara/issues/1900))
- **tests**: Drop scripted-LLM e2e and wiremock-based tests ([#1930](https://github.com/rararulab/rara/issues/1930)) ([#1933](https://github.com/rararulab/rara/issues/1933))

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
- **app**: Remove swagger-ui support ([#904](https://github.com/rararulab/rara/issues/904))
- **kernel**: Fix KernelHandle API inconsistencies ([#1027](https://github.com/rararulab/rara/issues/1027)) ([#1031](https://github.com/rararulab/rara/issues/1031))
- **kernel**: Type-state InboundMessage<Unresolved/Resolved> ([#1125](https://github.com/rararulab/rara/issues/1125)) ([#1134](https://github.com/rararulab/rara/issues/1134))
- **kernel**: SessionState transitions via methods ([#1143](https://github.com/rararulab/rara/issues/1143)) ([#1153](https://github.com/rararulab/rara/issues/1153))
- **workspace**: Extract browser/stt from kernel into driver crates ([#1146](https://github.com/rararulab/rara/issues/1146)) ([#1154](https://github.com/rararulab/rara/issues/1154))
- **kernel**: Clean up io.rs (typestate, constructors, dead code, hot path) ([#1180](https://github.com/rararulab/rara/issues/1180)) ([#1184](https://github.com/rararulab/rara/issues/1184))
- **channels**: 移除 TG reply keyboard + /model 失败日志 ([#1579](https://github.com/rararulab/rara/issues/1579)) ([#1580](https://github.com/rararulab/rara/issues/1580))
- **kernel**: Own trace build and save ([#1613](https://github.com/rararulab/rara/issues/1613)) ([#1614](https://github.com/rararulab/rara/issues/1614))
- **kernel**: Session-centric StreamHub event bus ([#1647](https://github.com/rararulab/rara/issues/1647)) ([#1652](https://github.com/rararulab/rara/issues/1652))
- **kernel**: Rename rara_message_id to rara_turn_id ([#1978](https://github.com/rararulab/rara/issues/1978)) ([#1991](https://github.com/rararulab/rara/issues/1991))

### Revert

- **channels**: Back out today's Telegram routing changes
- **kernel**: Back out remaining afternoon changes
- Restore April 13 changes previously rolled back

### Styling

- **channels**: Fix rustfmt formatting ([#723](https://github.com/rararulab/rara/issues/723)) ([#728](https://github.com/rararulab/rara/issues/728))
- **channels**: Fix formatting ([#1338](https://github.com/rararulab/rara/issues/1338)) ([#1339](https://github.com/rararulab/rara/issues/1339))

### Testing

- **channels**: Add unit and integration tests for TG streaming ([#38](https://github.com/rararulab/rara/issues/38))
- **app**: Add E2E test for anchor checkout conversation flow ([#198](https://github.com/rararulab/rara/issues/198))
- **channels**: Web adapter E2E tests via direct handler invocation ([#1178](https://github.com/rararulab/rara/issues/1178)) ([#1187](https://github.com/rararulab/rara/issues/1187))

<!-- generated by git-cliff -->
