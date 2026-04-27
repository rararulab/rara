# Issue #1935 — Implementation plan: RaraAgent on a single per-session WS

This plan replaces `pi-agent-core`'s `Agent` plus the two split WS endpoints
(`crates/channels/src/web.rs` chat WS, `crates/channels/src/web_session_events.rs`
events WS) with one persistent per-session WebSocket and a frontend
`RaraAgent` that owns the live message state. It collapses the dual
state-machine (`pi-agent-core` in-memory loop vs. tape DB truth) into one
state machine driven by tape events, eliminating the entire family of
races traced in #1601, #1731, #1732, #1849, #1867, #1877, #1880, #1923.

## 1. Goals & non-goals

### Goals
- **One WS** per session, lifecycle = sidebar selection (open on
  `switchSession`, close on session change/unmount). Replaces both
  `/api/v1/kernel/chat/ws` (per-turn) and
  `/api/v1/kernel/chat/events/{session_key}` (persistent).
- **One state-machine** on the frontend: `RaraAgent` owns `state.messages`
  and emits the seven `AgentEvent`s `<agent-interface>` subscribes to
  (`pi-web-ui/src/components/AgentInterface.ts:153-186`).
- **In-turn streaming preserved** — token-by-token text/thinking deltas
  still flow so UX does not regress.
- **`tape_appended` integrates with the turn**: sequenced after `done` on
  the same socket, so a single consumer cannot observe "stream finished
  but tape not yet visible" or vice-versa. This is what kills #1877:
  there is no second consumer (`use-session-events.ts`) racing the chat
  WS reload.
- **Drop `@mariozechner/pi-agent-core` dependency** entirely. Keep
  `@mariozechner/pi-ai` (for `Model`, `Message`, `calculateCost`,
  `ToolResultMessage` types) and `@mariozechner/pi-web-ui` (the renderer +
  `<pi-chat-panel>` shell).

### Non-goals (deferred)
- Server-side replay buffer changes. The existing
  `crates/channels/src/web_reply_buffer.rs` ring buffer keeps working;
  the new endpoint reuses it under the `subscribe_and_drain` pattern
  already proven in `web.rs:1073`.
- Steering / follow-up / `beforeToolCall` / `afterToolCall` hooks. These
  are unused by rara today (`PiChat.tsx` never calls them) — the plan
  documents that we do not implement them, callers will see them as
  no-ops if surfaced.
- Hot path for tool execution on the client. rara executes tools in the
  Rust kernel; the relay-tool shim in `rara-stream.ts:382-400` exists
  only to placate `pi-agent-core`'s post-stream loop. With `pi-agent-core`
  gone, the entire shim mechanism is deleted.
- Multi-tab fan-out semantics (two browser tabs on the same session).
  Today's `subscribe_and_drain` already handles this; we keep behavior.

## 2. Wire protocol

New endpoint: `GET /api/v1/kernel/chat/session/{session_key}` (WS upgrade).
Auth identical to existing chat WS: `Authorization: Bearer <token>` header
preferred, fall back to `?token=<…>` query param. Owner token verified
via `rara_kernel::auth::verify_owner_token`
(`crates/channels/src/web.rs:1003`). 401 on missing/invalid token.

### Lifecycle
1. **Client connects.** Server sends `hello` (frame #1) immediately —
   replaces `SessionEventFrame::Hello`
   (`web_session_events.rs:64`).
2. **Server starts forwarders.** Two tokio tasks pump into one mpsc
   draining to the socket (mirrors the existing pattern at
   `web.rs:1058-1147`):
   - kernel `StreamHub::subscribe_session_events` for in-turn deltas
   - notification bus `subscribe(NotificationFilter)` for `TapeAppended`
   - adapter-local `broadcast::Sender<WebEvent>` for `Typing`/`Error`/`Phase`
3. **Reply buffer drained** via `subscribe_and_drain`
   (`web.rs:1073`) — the buffered events for this session are flushed
   into the new socket before the live forwarders start, preserving
   the #1804 fix.
4. **Client sends inbound frames** (see below) at any time. No
   per-turn open/close — a long-lived connection carries N turns.
5. **Server emits `tape_appended` AFTER the matching `done`** for any
   tape append produced by that turn. The `Done` frame for stream X is
   already emitted by `stream_event_to_web_event` mapping
   `StreamEvent::StreamClosed` (`web.rs:384`), and the kernel publishes
   `TapeAppended` from `memory/service.rs:317` *after* the entry hits
   the DB. Because we now consume both buses on a single ordered mpsc,
   client sees `done` then `tape_appended` deterministically — no
   second-WS race.
6. **Reconnect.** Client reconnects with the same `session_key` and
   gets `hello` + replay-buffer drain again. Mid-turn frames the client
   missed are still in `web_reply_buffer` (TTL window) so a reconnect
   recovers without losing the assistant message.
7. **Close.** Client closes on session switch or unmount; server unhooks
   forwarders + endpoint registry (mirrors `web.rs:1280-1286`).

### Outbound frames (server → client)

Single discriminated union, `tag = "type"`, `rename_all = snake_case`:

| Frame | Replaces | When |
|---|---|---|
| `hello` | `web_session_events.rs:64` `SessionEventFrame::Hello` | on connect |
| `text_delta { text }` | `web.rs:114` `WebEvent::TextDelta` | LLM token |
| `reasoning_delta { text }` | `web.rs:116` `WebEvent::ReasoningDelta` | LLM thinking |
| `text_clear` | `web.rs:128` | kernel `StreamEvent::TextClear` |
| `tool_call_start { name, id, arguments }` | `web.rs:130` | kernel emits |
| `tool_call_end { id, result_preview, success, error }` | `web.rs:136` | kernel emits |
| `attachment { tool_call_id, mime_type, filename, data_base64 }` | `web.rs:206` | tool produces a file |
| `usage { input, output, cache_read, cache_write, total_tokens, cost, model }` | `web.rs:158` | per-turn |
| `turn_metrics { duration_ms, iterations, tool_calls, model }` | `web.rs:147` | per-turn |
| `turn_rationale { text }` | `web.rs:142` | per-turn |
| `progress { stage }` | `web.rs:144` | informational |
| `phase { phase }` | `web.rs:110` | informational |
| `typing` | `web.rs:108` | informational |
| `done` | `web.rs:235` | end of one assistant turn (NOT close socket) |
| `error { message }` | `web.rs:112` | per-turn or global |
| `plan_*`, `background_task_*`, `trace_ready` | `web.rs:167-200` | passthroughs |
| `approval_requested`, `approval_resolved` | `web.rs:221-233` | unchanged |
| `tape_appended { entry_id, role, timestamp }` | `web_session_events.rs:67` | after `done` for this turn, OR any out-of-turn append (background tasks, scheduled re-entries — #1849 path) |
| `message { content }` | `web.rs:106` | rare single-shot reply |

### Inbound frames (client → server)

Today the chat WS accepts a single text frame parsed as
`InboundPayload { content }` (`web.rs:390-403`). The new socket carries
N turns, so we wrap inbound in a tagged union:

| Frame | When | Behavior |
|---|---|---|
| `prompt { content: MessageContent }` | user submits | server runs `transcribe_audio_blocks` + `build_raw_platform_message` + `submit_message` (mirrors `web.rs:1207-1240`) |
| `abort` | user clicks stop | server interrupts current stream — same path as `POST /signals/{session_id}/interrupt` (`web.rs:558`) |

Backward-compat shim: server also accepts a bare JSON body matching the
old `InboundPayload` shape, treating it as `prompt`. This is for
test-fixtures only; the new client always sends `{"type":"prompt",…}`.

### Race elimination, by construction

The "in-turn streaming vs. tape append" race
(#1877/#1923) is killed because both event sources flow through one
**ordered** mpsc. Today `web.rs` consumes the StreamHub on a forwarder
task while `web_session_events.rs` consumes the notification bus on a
**different** WebSocket — order is undefined across sockets. After this
PR, the same forwarder consumes both and forwards in arrival order onto
one socket. Since `memory/service.rs:317` publishes `TapeAppended`
*after* the DB write, and `StreamEvent::StreamClosed` is what produces
`done`, the kernel's own emit order is `done`-then-`tape_appended` for
in-turn appends. Out-of-turn appends (background-task summaries) come
through with no preceding `done` — the client treats them as a tape
refetch trigger, identical to today's `useSessionEvents`.

### Error/disconnect handling
- **Connection lost mid-turn**: client reconnects (capped exp backoff,
  reuses `RECONNECT_BACKOFF_MS` constants from `rara-stream.ts:122`),
  re-receives buffered frames via `subscribe_and_drain`. No re-prompt
  needed — server is still running the turn against the same kernel
  session.
- **Server restart mid-turn**: replay buffer is in-memory only. Client
  shows error in the live card (`__stream_reconnect_failed` analog) and
  triggers a manual session reload to refetch tape. Same UX as today.
- **Auth failure**: 401 close, redirect to login (matches
  `rara-stream.ts:248`).

## 3. Backend changes

### New module
- **`crates/channels/src/web_session.rs`** — the persistent endpoint.
  - `pub struct WebSessionState { owner_token, owner_user_id, sink, stream_hub, endpoint_registry, adapter_events, reply_buffer, stt_service, shutdown_rx }` — the same fields as `WebAdapterState` (`web.rs:793`); reuse the existing struct by exporting it `pub(crate)` rather than duplicating.
  - `pub async fn session_ws_handler(WebSocketUpgrade, Path<String>, Query<TokenQuery>, HeaderMap, State<WebAdapterState>) -> Response` — auth + session-key parse + `ws.on_upgrade(handle_session_ws)`.
  - `async fn handle_session_ws(socket, key: SessionKey, state: WebAdapterState)` — three forwarders into one mpsc:
    1. adapter bus (Typing/Error/etc.) — copy of `web.rs:1090-1108`
    2. `StreamHub::subscribe_session_events` — copy of `web.rs:1110-1147`
    3. `handle.notification_bus().subscribe(NotificationFilter::default())` — copy of `web_session_events.rs:144-186` (filter to `TapeAppended` for this session, drop other variants)
  - Reuse `subscribe_and_drain` for reply-buffer drain on connect.
  - Recv task accepts the new tagged inbound union (`prompt` / `abort`).
  - Frame enum `SessionFrame` (outbound) — superset of `WebEvent` with `Hello` + `TapeAppended` variants. Use `#[serde(untagged)]` over a re-export of `WebEvent` plus the two new variants, OR (cleaner) extend `WebEvent` with `Hello` and `TapeAppended` and delete the separate `SessionEventFrame`.

  **Decision:** extend `WebEvent` with `Hello` + `TapeAppended { entry_id, role, timestamp }` variants, then delete `SessionEventFrame`. One discriminated union is simpler than two and the extra variants don't pollute the existing chat WS because that endpoint is being removed.

### Mount the route
- **`crates/channels/src/web.rs:530-561`** `WebAdapter::router()` — replace
  ```rust
  .route("/ws", get(ws_handler))
  .route("/events", get(sse_handler))
  .route("/messages", post(send_message_handler))
  .route("/signals/{session_id}/interrupt", post(interrupt_handler))
  …
  .merge(events_router)
  ```
  with
  ```rust
  .route("/session/{session_key}", get(web_session::session_ws_handler))
  .route("/signals/{session_id}/interrupt", post(interrupt_handler))
  ```
  Keep `/signals/.../interrupt` as a REST fallback for now (cheap; matches the existing abort path). SSE route deleted (see below).

### Files to delete
- **`crates/channels/src/web_session_events.rs`** — entirely. Its frame
  types fold into `WebEvent`; its handler is replaced by
  `web_session::session_ws_handler`.
- **`crates/channels/src/web.rs:986-1287`** — `ws_handler` + `handle_ws`
  (per-turn chat WS).
- **`crates/channels/src/web.rs:1293-…`** — `sse_handler` (lines 1293
  onwards through end of `sse_handler`). SSE has no consumer in
  `web/src` today (verified: `grep -r "EventSource" web/src` returns
  nothing) — delete rather than maintain.
- **`crates/channels/src/web.rs`** — `send_message_handler` and
  `SendMessageRequest`/`SendMessageResponse` (POST `/messages`). No
  consumer in `web/src` (`grep -rn "/api/v1/kernel/chat/messages" web/src`
  is empty).

### Files to keep (shared infrastructure)
- `web_reply_buffer.rs` — already-correct mechanism; reused as-is.
- `web.rs:262-386` (`platform_outbound_to_web_event`,
  `stream_event_to_web_event`) — pure mappers; both new + old endpoints
  consume them. Move to `web_frames.rs` if/when this PR creates that
  file; otherwise leave in place.
- Everything related to STT, identity resolution, endpoint registry,
  approval listener (`web.rs:697-786`), `WebAdapter::start`, the
  `ChannelAdapter` impl: untouched.

### Tests
Per `docs/guides/anti-patterns.md` ("Do NOT use mock repositories"), all
new tests use `testcontainers` + a real kernel. Add to
`crates/channels/tests/`:

- `web_session_smoke.rs` — boot kernel, connect WS, send `prompt`, drive
  a fake LLM driver to emit deltas + `done`, assert client sees
  `hello`, `text_delta*`, `done`, `tape_appended` in that order.
- `web_session_reconnect.rs` — connect, drop socket mid-turn, reconnect
  with same `session_key`, assert reply-buffer replay produces every
  buffered "important" event at most once.
- `web_session_abort.rs` — send `prompt`, send `abort` mid-turn, assert
  `error` frame with `aborted` reason and that the next `prompt` works.
- `web_session_out_of_turn_tape.rs` — submit a synthetic
  `KernelNotification::TapeAppended` (background-task path), assert
  client sees `tape_appended` with no surrounding `done` — the #1849
  invariant.

### AGENT.md
- **`crates/channels/AGENT.md`** — update (or create if missing) to
  reflect: one WS endpoint, single ordered mpsc, `web_reply_buffer` as
  the only mechanism between turns. Document the two new frame variants
  and the inbound `{type:"prompt"|"abort"}` shape.

## 4. Frontend changes

### New: `web/src/agent/rara-agent.ts`

Class implements the contract `<pi-chat-panel>` reads — see Goals + the
context summary above for the precise 12 reads / 8 calls / 7-event
subscribe / 3 rara-side methods.

```typescript
import type {
  Model, Message, AssistantMessage, ToolResultMessage, ImageContent,
} from '@mariozechner/pi-ai';

export type ThinkingLevel = 'off'|'minimal'|'low'|'medium'|'high'|'xhigh';

export interface RaraAgentTool { /* shape pi-web-ui's ChatPanel.setTools expects */ }

export interface RaraAgentState {
  systemPrompt: string;
  model: Model<any>;
  thinkingLevel: ThinkingLevel;
  tools: RaraAgentTool[];
  messages: AgentMessage[];           // pi-ai Message + UserMessageWithAttachments
  isStreaming: boolean;
  streamMessage: AssistantMessage | null;
  pendingToolCalls: Set<string>;
  error?: string;
}

type RaraAgentEvent =
  | { type: 'agent_start' }
  | { type: 'agent_end'; messages: AgentMessage[] }
  | { type: 'turn_start' }
  | { type: 'turn_end'; message: AgentMessage; toolResults: ToolResultMessage[] }
  | { type: 'message_start'; message: AgentMessage }
  | { type: 'message_update'; message: AgentMessage; assistantMessageEvent: any }
  | { type: 'message_end'; message: AgentMessage };

export class RaraAgent {
  state: RaraAgentState;                     // live mutable; PiChat.tsx mutates state.model directly
  sessionId: string | undefined;             // get/set; on set, switch socket subscription
  streamFn: never;                           // typed as never — kept for source-compat with AgentInterface
                                             // existing ===streamSimple guard, but we override the
                                             // setupSessionSubscription default by setting a sentinel
                                             // (see "Pi-web-ui shim" below)
  getApiKey?: (provider: string) => Promise<string|undefined>;

  setTools(t: RaraAgentTool[]): void;
  setModel(m: Model<any>): void;
  setThinkingLevel(l: ThinkingLevel): void;

  prompt(input: string | UserMessageWithAttachments): Promise<void>;
  abort(): void;
  appendMessage(m: AgentMessage): void;
  clearMessages(): void;                     // PiChat.tsx:499
  replaceMessages(ms: AgentMessage[]): void; // PiChat.tsx:564,611

  subscribe(fn: (e: RaraAgentEvent) => void): () => void;
}
```

### New: `web/src/agent/session-ws-client.ts`

Persistent WS client. Owns:
- one `WebSocket` per `sessionId` (rotates on `sessionId` set)
- inbound frame parser (the `WebEvent` union, extended with `hello` + `tape_appended`)
- outbound frame builder (`prompt` / `abort`)
- reconnect with capped exp backoff — reuses the constants from
  `rara-stream.ts:122` (move them to a shared `web/src/agent/backoff.ts`).
- a small event emitter that the `RaraAgent` translates into
  `RaraAgentEvent`s.

The WS URL builder `buildWsUrl` (currently at `rara-stream.ts:225`) is
**moved verbatim** into `session-ws-client.ts`. It already handles the
three resolution cases (override / `BASE_URL` / page-derived) and the
auth-token redirect.

### `RaraAgent` ↔ pi-web-ui contract — exact mapping

| pi-web-ui call site | What it expects | RaraAgent answer |
|---|---|---|
| `AgentInterface.ts:138` `session.streamFn === streamSimple` | sentinel comparison gate | set `streamFn` on construction to a unique `Symbol`-keyed function so the comparison is false and `setupSessionSubscription` skips proxy injection |
| `AgentInterface.ts:146` `if (!this.session.getApiKey)` | optional resolver | leave undefined; rara manages keys server-side (`PiChat.tsx:894` `onApiKeyRequired: async () => true`) |
| `AgentInterface.ts:153` `session.subscribe(fn)` | returns unsubscribe | implement |
| `AgentInterface.ts:156-184` event types | `agent_start \| agent_end \| turn_start \| turn_end \| message_start \| message_update \| message_end` | emit exactly these |
| `AgentInterface.ts:179` `session.state.isStreaming` | bool | maintain |
| `AgentInterface.ts:181` `ev.message` on `message_update` | latest streaming AssistantMessage snapshot | rebuild from running content blocks (mirror `rara-stream.ts:196 buildPartial`) |
| `AgentInterface.ts:216` `session.state.isStreaming` | reentrancy guard | maintain |
| `AgentInterface.ts:219` `session.state.model` | non-null after init | enforced by PiChat init flow already |
| `AgentInterface.ts:258, 260` `session.prompt(...)` | starts a turn | send `{type:"prompt", content: …}` after building `MessageContent` from the input |
| `AgentInterface.ts:267` `state.messages` | array | maintain |
| `AgentInterface.ts:270` iterate `messages[].role === 'toolResult'` | toolResult messages exist | append `toolResult` AgentMessages to `state.messages` on `tool_call_end` (mirrors today's relay-tool resolution at `rara-stream.ts:716-752`, but inline — no shim needed) |
| `AgentInterface.ts:281` `state.pendingToolCalls` | `Set<string>` | maintain (add on `tool_call_start`, delete on `tool_call_end`) |
| `AgentInterface.ts:374` `session.abort()` | aborts | send `{type:"abort"}`, locally fire error termination if no server response in N ms |
| `AgentInterface.ts:379` `session.setModel(model)` | updates state | implement |
| `AgentInterface.ts:385` `session.setThinkingLevel(level)` | updates state | implement |
| `AgentInterface.ts:280-294` `state.tools` / `pendingToolCalls` / `streamMessage` | bag of UI inputs | maintain |
| `ChatPanel.ts:144` `agent.setTools(tools)` | accepts tools array | implement (tools are not executed client-side; they're decoration for the renderer) |

### `PiChat.tsx` migration

| Current line | Change |
|---|---|
| `PiChat.tsx:17` `import { Agent } from '@mariozechner/pi-agent-core'` | `import { RaraAgent as Agent } from '@/agent/rara-agent'` (alias keeps the rest of the file untouched) |
| `PiChat.tsx:55` `import { createRaraStreamFn } from '@/adapters/rara-stream'` | delete |
| `PiChat.tsx:80` `import { useSessionEvents } from '@/hooks/use-session-events'` | delete |
| `PiChat.tsx:652-658` `useSessionEvents({ sessionKey, onTapeAppended })` | delete; behavior is now in `RaraAgent`. `RaraAgent` re-fires `replaceMessages` on `tape_appended` only when `!state.isStreaming` (preserve the #1877 guard) |
| `PiChat.tsx:812-846` `new Agent({ streamFn, convertToLlm, sessionId })` | `new RaraAgent({ sessionId: initialSession.key, observer: (sessionKey, event) => { liveRunStore.publish(sessionKey, event); … } })` — `RaraAgent` exposes the same observer hook the rara-stream factory had at `rara-stream.ts:837-842` |
| `PiChat.tsx:27` `defaultConvertToLlm` | unused (server owns the LLM call); delete the import. Stays out of the `RaraAgent` constructor surface |
| `PiChat.tsx:17` etc. — anywhere else that imports from `@mariozechner/pi-agent-core` | replaced. The only types still needed (`AgentEvent`, `AgentTool`) come from `@/agent/rara-agent` |
| `pi-chat-messages.ts` references to `pi-agent-core` types | re-target to `@/agent/rara-agent` |

### Files to delete
- `web/src/adapters/rara-stream.ts` (964 lines)
- `web/src/adapters/__tests__/rara-stream.test.ts` (if present — `ls` showed `__tests__/`)
- `web/src/hooks/use-session-events.ts` (137 lines)

`buildWsUrl` is **moved** into `web/src/agent/session-ws-client.ts`; do
not delete the function, only its current home.

There is no `ws-base-url.ts` in the repo (verified by grep) — the
question in the brief was speculative.

### `package.json`
Remove `"@mariozechner/pi-agent-core"` from `web/package.json`.

### New vitest tests
`web/src/agent/__tests__/`:

- `rara-agent.test.ts` — drive the agent with a mock `SessionWsClient`,
  assert: tape-appended → replaceMessages call shape; abort flips
  `isStreaming` and emits `agent_end`; reconnect resumes without
  duplicating frames; `tool_call_start` adds `pendingToolCalls`,
  `tool_call_end` removes + appends `toolResult`.
- `session-ws-client.test.ts` — feed JSON frames into the parser, assert
  the emitter ordering. Use `mock-socket` (already in
  `web/package.json` if present; otherwise add as `devDependency`).
- Update `web/e2e/` Playwright spec for chat (if it exists; check
  `web/playwright.config.ts`) — should pass unchanged because the
  `<pi-chat-panel>` UI is unchanged.

## 5. Migration / cutover

### Edit order inside the PR
1. **Backend** — add `web_session.rs`, extend `WebEvent` with `Hello`/`TapeAppended`, mount the new route. Keep old chat-WS + events-WS endpoints temporarily (for the dev frontend to keep working). `cargo check -p rara-channels`.
2. **Frontend** — write `agent/rara-agent.ts` + `agent/session-ws-client.ts` + tests; do not wire into `PiChat.tsx` yet. `cd web && npm run build` + `npm test`.
3. **Wire** — switch `PiChat.tsx` import from `pi-agent-core` to `rara-agent`. Delete `useSessionEvents` call. Build, manual smoke against remote backend per `docs/guides/debug.md`.
4. **Delete** old endpoints: `web_session_events.rs`, the `ws_handler`/`sse_handler`/`send_message_handler` blocks in `web.rs`, the routes in `router()`.
5. **Delete** old frontend files: `rara-stream.ts`, `use-session-events.ts`, drop the `pi-agent-core` package dep.
6. **AGENT.md** — update.
7. **prek run --all-files** — clippy + fmt + doc + tests.

### Integration testing during development
The remote backend runs from `~/code/rararulab/rara` on `raratekiAir`
(per `docs/guides/debug.md`). Push the backend half to a feature
branch, `ssh local-rara && pkill -f "target/debug/rara " && nohup just run …`
**after confirming with the user**. Frontend dev runs locally with
`VITE_API_URL=http://10.0.0.183:25555 bun run dev` against the worktree
backend. Logdy at `http://10.0.0.183:8080` is the live trace.

### Smoke test plan before pushing
- New session, single-shot text prompt, see deltas → `done` →
  `tape_appended`, message persists across reload.
- Tool call (e.g. memory tool), confirm `tool_call_start`/`_end` paint
  the chip card and a `toolResult` lands in messages.
- Stop button mid-stream, confirm clean error + ability to send next prompt.
- Switch sessions twice in quick succession (#1867 repro), confirm no
  cross-session message bleed.
- Background-task path (#1849 repro): trigger a scheduled re-entry
  (or simulate via kernel notification), confirm `tape_appended`
  arrives with no preceding `done` and the tape refetches.
- Reload mid-stream (#1877 repro): turn finishes server-side while page
  is closed, reconnect drains buffer and renders the assistant message.

## 6. Risk register

| Risk | P | I | Mitigation |
|---|---|---|---|
| `<pi-chat-panel>` reads a state field we missed (e.g. internal undocumented prop) | M | H | Run pi-web-ui's example fixtures against `RaraAgent` before wiring rara. Grep `pi-web-ui/src/**/*.ts` for every `agent.` / `session.` access — already done for the brief; spot-check before merging. The `streamFn === streamSimple` sentinel comparison at `AgentInterface.ts:138` is the most fragile — covered by the symbol shim. |
| pi-web-ui has internal expectations on event ordering (e.g. `message_update` before `message_end`) | M | M | Mirror `pi-agent-core`'s emit order: `agent_start` → `turn_start` → `message_start` → `message_update*` → `message_end` → `turn_end` → `agent_end`. Verify by recording events from the current `pi-agent-core` against a smoke prompt, then asserting `RaraAgent` produces an identical trace. |
| Backend persistent WS leaks resources on disconnect | L | M | Reuse the existing `tokio::select!{ send_task; recv_task; }` + `forwarder.abort()` shape from `web.rs:1273-1279`; keep `unregister_endpoint` on close. Add a regression test that connects + drops + asserts `endpoint_registry.is_registered` is false within 100 ms. |
| Browser reload mid-turn loses state | M | M | Reply buffer + reconnect already handles this; test #1877 repro explicitly. |
| `pi-web-ui` upgrade breaks our shim | M | H | Pin `@mariozechner/pi-web-ui` and `@mariozechner/pi-ai` to exact versions in `package.json`. Add an integration test that imports `Agent` type from pi-web-ui types-only and structurally asserts `RaraAgent` is assignable to it. If a future bump breaks structure, the build fails loud. |
| Tape append race during very fast turns (`done` and `tape_appended` could arrive faster than the React commit cycle) | L | L | RaraAgent buffers `tape_appended` while `state.isStreaming === true` until the matching `done` flips it false (mirrors `PiChat.tsx:655` guard, but inside the agent now). |
| Out-of-turn `tape_appended` (background tasks) lands while user is composing | L | L | Same path as today (`useSessionEvents` already triggers reload); preserve via `RaraAgent.replaceMessages` when not streaming. |

## 7. Verification checklist

- [ ] `cargo check -p rara-channels` clean
- [ ] `cd web && npm run build` clean (no TS errors)
- [ ] `cd web && npm test` all green
- [ ] `prek run --all-files` clean (clippy + fmt + doc warnings)
- [ ] Smoke: send / stop / model picker / thinking selector / attachments / tool calls / artifacts all work locally against remote backend
- [ ] Reconnect mid-turn recovers without data loss (manual)
- [ ] Background-task summaries (#1849) still trigger refresh
- [ ] Session switching (`switchSession`) loads correct history, no cross-session bleed (#1867)
- [ ] Send a turn, hard-reload at first delta, confirm assistant message appears after reload (#1877)
- [ ] All existing web tests still pass
- [ ] No new clippy warnings, fmt clean
- [ ] AGENT.md updated for `crates/channels`
- [ ] `gh pr checks <N> --watch` reaches green before requesting review

## 8. LOC budget & timing estimate

| Area | Add | Delete | Net |
|---|---|---|---|
| `crates/channels/src/web_session.rs` | ~300 | 0 | +300 |
| `crates/channels/src/web.rs` | ~10 | ~600 | −590 |
| `crates/channels/src/web_session_events.rs` | 0 | 215 | −215 |
| `crates/channels/AGENT.md` | ~30 | ~10 | +20 |
| `crates/channels/tests/web_session_*.rs` | ~400 | 0 | +400 |
| `web/src/agent/rara-agent.ts` | ~350 | 0 | +350 |
| `web/src/agent/session-ws-client.ts` | ~250 | 0 | +250 |
| `web/src/agent/__tests__/*.ts` | ~250 | 0 | +250 |
| `web/src/adapters/rara-stream.ts` | 0 | 964 | −964 |
| `web/src/adapters/__tests__/rara-stream.test.ts` | 0 | ~200 | −200 |
| `web/src/hooks/use-session-events.ts` | 0 | 137 | −137 |
| `web/src/pages/PiChat.tsx` | ~5 | ~25 | −20 |
| `web/package.json` | 0 | 1 | −1 |
| **Total** | ~1595 | ~2152 | **−557** |

This is a net deletion of ~550 lines while adding the test suite that
proves the race classes are gone.

**Time:** 3.5 working days for one engineer.
- Day 1: backend `web_session.rs` + tests against testcontainers
- Day 2: frontend `RaraAgent` + `SessionWsClient` + vitest suite
- Day 3: wire into `PiChat.tsx`, manual smoke against remote, fix
  whatever pi-web-ui internal expectation we missed
- Day 0.5: code review loop + CI debugging

## 9. Open questions for the user

1. **Inbound frame compatibility window.** Should the new endpoint
   accept the legacy bare `InboundPayload` JSON (as a transitional
   hack, in case rara-cli or some script speaks it) or hard-fail on
   anything that's not `{type:"prompt"|"abort"}`? Recommendation:
   **hard-fail** — there's no other client and shimming would create
   the same kind of dual contract this PR is removing.

2. **`POST /signals/{session_id}/interrupt` survival.** Plan keeps it
   for now because the abort path already routes through it. Should
   abort go exclusively over the new WS (cleaner, one channel) and
   the REST endpoint be deleted in this same PR? Recommendation:
   **delete the REST endpoint too** if grep confirms no consumer in
   `web/src` or `crates/cli`.

3. **Tools array shape.** `pi-chat-panel` calls `agent.setTools(tools)`
   with an array of pi-web-ui's `AgentTool`. Today these tools are
   never executed client-side (kernel runs them). The plan keeps the
   tools array purely as renderer decoration. Confirm this is the
   intended posture and we're not preserving any client-side tool
   execution path.
