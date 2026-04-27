/*
 * Copyright 2025 Rararulab
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

/**
 * `RaraAgent` — frontend `Agent` replacement for `<pi-chat-panel>`.
 *
 * Phase (c) of #1935. Owns `state.messages` and translates a stream of
 * backend `WebFrame`s (from `SessionWsClient`) into the seven
 * `RaraAgentEvent` variants `<agent-interface>` reads at
 * `AgentInterface.ts:153-186`. The kernel runs the LLM and tools — this
 * class is purely a state machine + event translator on the client.
 *
 * Emission ordering for one text-only assistant turn matches
 * pi-agent-core's `runAgentLoop` exactly (see
 * `node_modules/.../pi-agent-core/dist/agent-loop.js:42-56,140-218`):
 *
 *   agent_start
 *   turn_start
 *   message_start (user)        ──┐
 *   message_end   (user)        ──┘ per prompt message
 *   message_start (assistant partial, on first content frame)
 *   message_update*             (per text_delta / reasoning_delta / tool_call_*)
 *   message_end   (assistant final)
 *   turn_end      (assistant, toolResults: [...])
 *   agent_end     (messages snapshot)
 *
 * Tool calls add per-result `message_start` / `message_end` for the
 * `toolResult` AgentMessage, mirroring pi-agent-core's
 * `executeToolCall*` (`agent-loop.js:386-387`).
 */

import { calculateCost } from '@mariozechner/pi-ai';
import type {
  Api,
  AssistantMessage,
  AssistantMessageEvent,
  ImageContent,
  Model,
  TextContent,
  ThinkingContent,
  ToolCall,
  ToolResultMessage,
  Usage,
} from '@mariozechner/pi-ai';
import type { UserMessageWithAttachments } from '@mariozechner/pi-web-ui';

import {
  type LifecycleEvent,
  type PromptContent,
  type PromptContentBlock,
  SessionWsClient,
  type WebFrame,
} from './session-ws-client';
import type {
  AgentMessage,
  AgentTool,
  RaraAgentEvent,
  RaraAgentState,
  ThinkingLevel,
} from './types';

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

export interface RaraAgentOptions {
  /** Initial session key. May be set/changed later via `agent.sessionId = …`. */
  sessionId?: string;
  /** Initial model. Required for `prompt()` to succeed; usually set by `PiChat.tsx`. */
  model?: Model<Api>;
  /** Initial reasoning level. */
  thinkingLevel?: ThinkingLevel;
  /** Initial system prompt (recorded for parity; not sent — server owns the prompt). */
  systemPrompt?: string;
  /**
   * Override factory for the WS client. Tests inject a fake here to feed
   * frames deterministically without a real WebSocket.
   */
  clientFactory?: (sessionKey: string) => SessionWsClient;
  /**
   * Receives raw `WebFrame`s alongside the agent-event projection. Mirrors
   * the legacy `WebEventObserver` from `rara-stream.ts` so the live-card
   * store can keep observing in-flight frames during the migration window.
   */
  observer?: (sessionKey: string, frame: WebFrame) => void;
  /**
   * Reserved for source-compat with pi-agent-core's `AgentOptions` —
   * `<pi-chat-panel>` is configured today with a `convertToLlm` arg that
   * folds `AgentMessage[]` into pi-ai's `Message[]` for the LLM call.
   * RaraAgent never invokes it (the kernel runs the LLM), but accepting
   * the field keeps the call site at `PiChat.tsx:844` unchanged.
   */
  convertToLlm?: unknown;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Build a zeroed Usage object for the initial pre-`usage`-frame snapshot. */
function emptyUsage(): Usage {
  return {
    input: 0,
    output: 0,
    cacheRead: 0,
    cacheWrite: 0,
    totalTokens: 0,
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
  };
}

/** Snapshot the running assistant message — caller-owned, freshly allocated. */
function buildPartial(
  model: Model<Api>,
  content: (TextContent | ThinkingContent | ToolCall)[],
  usage: Usage,
): AssistantMessage {
  return {
    role: 'assistant',
    content: [...content],
    api: model.api,
    provider: model.provider,
    model: model.id,
    usage,
    stopReason: 'stop',
    timestamp: Date.now(),
  };
}

/**
 * Coerce caller-supplied `prompt(input)` argument into a wire-ready
 * `PromptContent` plus the `AgentMessage` to record locally.
 *
 * - String → plain text on the wire, plain `UserMessage` locally.
 * - `UserMessageWithAttachments` → multimodal blocks on the wire,
 *   record the original message so the renderer keeps the attachment chips.
 */
function prepareUserInput(input: string | UserMessageWithAttachments): {
  wire: PromptContent;
  local: AgentMessage;
} {
  if (typeof input === 'string') {
    return {
      wire: input,
      local: { role: 'user', content: input, timestamp: Date.now() },
    };
  }

  const blocks: PromptContentBlock[] = [];
  if (typeof input.content === 'string') {
    blocks.push({ type: 'text', text: input.content });
  } else {
    for (const c of input.content) {
      if (c.type === 'text') {
        blocks.push({ type: 'text', text: c.text });
      } else if (c.type === 'image') {
        // pi-ai uses { mimeType, data }; rara wire format uses { media_type, data }.
        blocks.push({ type: 'image_base64', media_type: c.mimeType, data: c.data });
      }
    }
  }
  for (const att of input.attachments ?? []) {
    if (att.type === 'document') {
      blocks.push({
        type: 'file_base64',
        media_type: att.mimeType,
        data: att.content,
        filename: att.fileName,
      });
    }
  }

  return { wire: blocks, local: input };
}

// ---------------------------------------------------------------------------
// RaraAgent
// ---------------------------------------------------------------------------

/**
 * Lifecycle-aware Agent surface consumed by `<pi-chat-panel>`.
 *
 * The single mutable `state` object is held internally and exposed via
 * a getter that always returns the same reference — `PiChat.tsx` mutates
 * `state.model` directly, and `<agent-interface>` reads `state.isStreaming`
 * many times per frame, so a fresh-snapshot getter would break both.
 */
export class RaraAgent {
  private readonly _state: RaraAgentState;
  private readonly listeners = new Set<(e: RaraAgentEvent) => void>();
  private readonly clientFactory: (sessionKey: string) => SessionWsClient;
  private readonly observer: ((sessionKey: string, frame: WebFrame) => void) | undefined;

  private _sessionId: string | undefined;
  private client: SessionWsClient | null = null;
  private clientUnsubscribeFrame: (() => void) | null = null;
  private clientUnsubscribeLifecycle: (() => void) | null = null;

  /** In-flight assistant content blocks — folded into a fresh snapshot each emit. */
  private streamingContent: (TextContent | ThinkingContent | ToolCall)[] = [];
  /** Has `message_start` been emitted for the in-flight assistant message? */
  private assistantStarted = false;
  /** Tool-call ids whose `tool_call_start` arrived but `tool_call_end` did not. */
  private readonly pendingTurnToolResults: ToolResultMessage[] = [];
  /** Per-tool-call attachments buffered until the matching `tool_call_end`. */
  private readonly pendingAttachments = new Map<
    string,
    { mime_type: string; filename: string | null; data_base64: string }[]
  >();
  private currentUsage: Usage = emptyUsage();
  /** True between `agent_start` and the matching `agent_end`/`error`. */
  private turnActive = false;

  // ------------------------------------------------------------------
  // Source-compat surface for pi-web-ui
  //
  // `<agent-interface>` performs identity checks on `streamFn` and
  // `getApiKey` (`AgentInterface.ts:138, 146`). Setting `streamFn` to a
  // unique sentinel function keeps the `=== streamSimple` comparison
  // false, so pi-web-ui's `setupSessionSubscription` skips its own
  // proxy-wrapping default. `getApiKey` is intentionally `undefined`
  // until pi-web-ui assigns one — rara serves keys server-side and
  // never invokes it, but accepting writes keeps the host's identity
  // assignment a no-op rather than a runtime failure.
  // ------------------------------------------------------------------

  /** Sentinel never invoked — only present so identity checks at `AgentInterface.ts:138` see a value distinct from `streamSimple`. */
  streamFn: (...args: unknown[]) => never = () => {
    throw new Error('RaraAgent.streamFn is a sentinel and must not be called');
  };

  /** Optional API-key resolver; assigned by `<agent-interface>`, never invoked by RaraAgent. */
  getApiKey?: (provider: string) => Promise<string | undefined> | string | undefined;

  constructor(opts: RaraAgentOptions = {}) {
    this._state = {
      systemPrompt: opts.systemPrompt ?? '',
      // Cast through unknown — caller always sets `state.model` before
      // the first `prompt()`; non-null after init is enforced by the
      // pi-web-ui flow at `PiChat.tsx:844-855`.
      model: (opts.model ?? null) as unknown as Model<Api>,
      thinkingLevel: opts.thinkingLevel ?? 'off',
      tools: [],
      messages: [],
      isStreaming: false,
      streamMessage: null,
      pendingToolCalls: new Set(),
      error: undefined,
    };
    this.clientFactory =
      opts.clientFactory ?? ((sessionKey: string) => new SessionWsClient({ sessionKey }));
    this.observer = opts.observer;
    if (opts.sessionId) this.sessionId = opts.sessionId;
  }

  // ------------------------------------------------------------------
  // pi-web-ui contract surface
  // ------------------------------------------------------------------

  get state(): RaraAgentState {
    return this._state;
  }

  get sessionId(): string | undefined {
    return this._sessionId;
  }

  set sessionId(value: string | undefined) {
    if (value === this._sessionId) return;
    this.teardownClient();
    this._sessionId = value;
    if (value) this.setupClient(value);
  }

  subscribe(fn: (e: RaraAgentEvent) => void): () => void {
    this.listeners.add(fn);
    return () => {
      this.listeners.delete(fn);
    };
  }

  setTools(tools: AgentTool[]): void {
    this._state.tools = tools;
  }

  setModel(m: Model<Api>): void {
    this._state.model = m;
  }

  setThinkingLevel(level: ThinkingLevel): void {
    this._state.thinkingLevel = level;
  }

  /**
   * Append a message externally. Used by `ArtifactsRuntimeProvider`
   * (`PiChat.tsx`) to inject `artifact` AgentMessages without a turn.
   *
   * Emits the `message_start` / `message_end` pair so renderers see the
   * insertion identically to a kernel-driven append.
   */
  appendMessage(message: AgentMessage): void {
    this._state.messages.push(message);
    this.emit({ type: 'message_start', message });
    this.emit({ type: 'message_end', message });
  }

  /** Replace the entire message list — used by tape reload after `tape_appended`. */
  replaceMessages(messages: AgentMessage[]): void {
    this._state.messages = messages;
  }

  /** Drop all messages — used on session switch + `clearMessages` button. */
  clearMessages(): void {
    this._state.messages = [];
  }

  // ------------------------------------------------------------------
  // Turn control
  // ------------------------------------------------------------------

  async prompt(input: string | UserMessageWithAttachments): Promise<void> {
    if (this._state.isStreaming) {
      // Mirror pi-agent-core: throwing here surfaces "agent already
      // processing" via `<agent-interface>`'s reentrancy guard at
      // `AgentInterface.ts:216`. In practice the host short-circuits
      // before calling `prompt`, so this is defense-in-depth.
      throw new Error('Agent is already processing');
    }
    if (!this._state.model) {
      throw new Error('No model configured');
    }
    if (!this.client) {
      throw new Error('No active session — set sessionId before prompting');
    }

    const { wire, local } = prepareUserInput(input);

    // Record the user message + emit the user-message lifecycle pair.
    // Order mirrors pi-agent-core (`agent-loop.js:42-56`).
    this._state.messages.push(local);
    this._state.error = undefined;
    this._state.isStreaming = true;
    this.turnActive = true;
    this.assistantStarted = false;
    this.streamingContent = [];
    this.currentUsage = emptyUsage();
    this.pendingTurnToolResults.length = 0;

    this.emit({ type: 'agent_start' });
    this.emit({ type: 'turn_start' });
    this.emit({ type: 'message_start', message: local });
    this.emit({ type: 'message_end', message: local });

    if (!this.client.prompt(wire)) {
      // Socket isn't open yet (still mid-handshake or reconnecting).
      // Surface as a clean error termination — let the host retry.
      this.terminateWithError('WebSocket not ready');
    }
  }

  /**
   * Abort the current turn. Best-effort: sends `abort` to the backend
   * and locally terminates the in-flight assistant message with
   * `stopReason: "aborted"`. The backend reply that closes the stream
   * arrives separately; we don't wait for it.
   */
  abort(): void {
    if (!this.turnActive) return;
    this.client?.abort();
    this.terminateWithError('Aborted by user', 'aborted');
  }

  // ------------------------------------------------------------------
  // WS client wiring
  // ------------------------------------------------------------------

  private setupClient(sessionKey: string): void {
    const client = this.clientFactory(sessionKey);
    this.client = client;
    this.clientUnsubscribeFrame = client.onFrame((frame) => this.handleFrame(sessionKey, frame));
    this.clientUnsubscribeLifecycle = client.onLifecycle((event) => this.handleLifecycle(event));
    client.connect();
  }

  private teardownClient(): void {
    this.clientUnsubscribeFrame?.();
    this.clientUnsubscribeLifecycle?.();
    this.clientUnsubscribeFrame = null;
    this.clientUnsubscribeLifecycle = null;
    this.client?.disconnect();
    this.client = null;
  }

  private handleLifecycle(event: LifecycleEvent): void {
    if (event.type === 'closed' && event.reason === 'reconnect_exhausted' && this.turnActive) {
      this.terminateWithError('WebSocket reconnect failed');
    }
  }

  private handleFrame(sessionKey: string, frame: WebFrame): void {
    // Surface raw frames first so external observers (live-card store)
    // see them even if the in-class projection below ignores the variant.
    if (this.observer) {
      try {
        this.observer(sessionKey, frame);
      } catch (err) {
        console.warn('RaraAgent: observer threw', err);
      }
    }

    switch (frame.type) {
      case 'hello':
      case 'typing':
      case 'phase':
      case 'progress':
      case 'turn_rationale':
      case 'turn_metrics':
      case 'plan_created':
      case 'plan_progress':
      case 'plan_replan':
      case 'plan_completed':
      case 'background_task_started':
      case 'background_task_done':
      case 'trace_ready':
      case 'approval_requested':
      case 'approval_resolved':
        // Informational — surfaced via `observer` only.
        return;

      case 'text_delta':
        this.handleTextDelta(frame.text);
        return;
      case 'reasoning_delta':
        this.handleReasoningDelta(frame.text);
        return;
      case 'text_clear':
        // Drop in-flight text content so the next delta starts fresh.
        this.streamingContent = this.streamingContent.filter((b) => b.type !== 'text');
        if (this.assistantStarted) this.emitAssistantUpdate({ type: 'text_clear' } as never);
        return;

      case 'tool_call_start':
        this.handleToolCallStart(frame);
        return;
      case 'tool_call_end':
        this.handleToolCallEnd(frame);
        return;
      case 'attachment':
        this.handleAttachment(frame);
        return;

      case 'usage': {
        const next: Usage = {
          input: frame.input,
          output: frame.output,
          cacheRead: frame.cache_read,
          cacheWrite: frame.cache_write,
          totalTokens: frame.total_tokens,
          cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
        };
        if (this._state.model) next.cost = calculateCost(this._state.model, next);
        this.currentUsage = next;
        return;
      }

      case 'message': {
        // Single-shot complete message — fold into a text block + done.
        this.appendOrExtendText(frame.content);
        this.finalizeAssistantTurn('stop');
        return;
      }
      case 'done':
        this.finalizeAssistantTurn('stop');
        return;
      case 'error':
        this.terminateWithError(frame.message);
        return;

      case 'tape_appended':
        // Out-of-band tape mutation. RaraAgent does not eagerly refetch
        // here — `PiChat.tsx` owns the reload policy (today via
        // `useSessionEvents`; phase d folds it into RaraAgent). Emitted
        // only via `observer` so the host can decide.
        return;
    }
  }

  // ------------------------------------------------------------------
  // Stream-frame handlers
  // ------------------------------------------------------------------

  private handleTextDelta(text: string): void {
    this.ensureAssistantStarted();
    const last = this.streamingContent[this.streamingContent.length - 1];
    let block: TextContent;
    if (last && last.type === 'text') {
      block = last;
      block.text += text;
    } else {
      block = { type: 'text', text };
      this.streamingContent.push(block);
    }
    this.emitAssistantUpdate({
      type: 'text_delta',
      contentIndex: this.streamingContent.indexOf(block),
      delta: text,
      partial: this.snapshot(),
    } as unknown as AssistantMessageEvent);
  }

  private handleReasoningDelta(text: string): void {
    this.ensureAssistantStarted();
    const last = this.streamingContent[this.streamingContent.length - 1];
    let block: ThinkingContent;
    if (last && last.type === 'thinking') {
      block = last;
      block.thinking += text;
    } else {
      block = { type: 'thinking', thinking: text };
      this.streamingContent.push(block);
    }
    this.emitAssistantUpdate({
      type: 'thinking_delta',
      contentIndex: this.streamingContent.indexOf(block),
      delta: text,
      partial: this.snapshot(),
    } as unknown as AssistantMessageEvent);
  }

  private handleToolCallStart(frame: {
    id: string;
    name: string;
    arguments: Record<string, unknown>;
  }): void {
    this.ensureAssistantStarted();
    const toolCall: ToolCall = {
      type: 'toolCall',
      id: frame.id,
      name: frame.name,
      arguments: frame.arguments,
    };
    this.streamingContent.push(toolCall);

    // pendingToolCalls is read by `<agent-interface>` to gate spinner
    // chips. We must replace the Set rather than mutate so React-style
    // identity checks fire (`agent.js:301-305` rebuilds it the same way).
    const next = new Set(this._state.pendingToolCalls);
    next.add(frame.id);
    this._state.pendingToolCalls = next;

    this.emitAssistantUpdate({
      type: 'toolcall_start',
      contentIndex: this.streamingContent.length - 1,
      partial: this.snapshot(),
    } as unknown as AssistantMessageEvent);
  }

  private handleToolCallEnd(frame: {
    id: string;
    result_preview: string;
    success: boolean;
    error: string | null;
  }): void {
    const idx = this.streamingContent.findIndex((c) => c.type === 'toolCall' && c.id === frame.id);
    let toolCallName = 'unknown';
    if (idx >= 0) {
      const tc = this.streamingContent[idx] as ToolCall;
      toolCallName = tc.name;
      this.emitAssistantUpdate({
        type: 'toolcall_end',
        contentIndex: idx,
        toolCall: tc,
        partial: this.snapshot(),
      } as unknown as AssistantMessageEvent);
    }

    // Build the toolResult AgentMessage.
    const text = frame.error ?? frame.result_preview;
    const content: (TextContent | ImageContent)[] = [{ type: 'text', text }];
    const atts = this.pendingAttachments.get(frame.id);
    if (atts) {
      for (const att of atts) {
        if (att.mime_type.startsWith('image/')) {
          content.push({ type: 'image', data: att.data_base64, mimeType: att.mime_type });
        } else {
          const label = att.filename ?? 'attachment';
          const href = `data:${att.mime_type};base64,${att.data_base64}`;
          content.push({ type: 'text', text: `[${label}](${href})` });
        }
      }
      this.pendingAttachments.delete(frame.id);
    }

    const toolResult: ToolResultMessage = {
      role: 'toolResult',
      toolCallId: frame.id,
      toolName: toolCallName,
      content,
      isError: !frame.success,
      timestamp: Date.now(),
    };
    this.pendingTurnToolResults.push(toolResult);

    const nextPending = new Set(this._state.pendingToolCalls);
    nextPending.delete(frame.id);
    this._state.pendingToolCalls = nextPending;
  }

  private handleAttachment(frame: {
    tool_call_id: string | null;
    mime_type: string;
    filename: string | null;
    data_base64: string;
  }): void {
    if (!frame.tool_call_id) return;
    const list = this.pendingAttachments.get(frame.tool_call_id) ?? [];
    list.push({
      mime_type: frame.mime_type,
      filename: frame.filename,
      data_base64: frame.data_base64,
    });
    this.pendingAttachments.set(frame.tool_call_id, list);
  }

  // ------------------------------------------------------------------
  // Assistant-message lifecycle helpers
  // ------------------------------------------------------------------

  private ensureAssistantStarted(): void {
    if (this.assistantStarted) return;
    this.assistantStarted = true;
    const partial = this.snapshot();
    this._state.streamMessage = partial;
    this._state.messages.push(partial);
    this.emit({ type: 'message_start', message: partial });
  }

  private appendOrExtendText(text: string): void {
    this.ensureAssistantStarted();
    const last = this.streamingContent[this.streamingContent.length - 1];
    if (last && last.type === 'text') {
      last.text += text;
    } else {
      this.streamingContent.push({ type: 'text', text });
    }
  }

  private snapshot(): AssistantMessage {
    return buildPartial(this._state.model, this.streamingContent, this.currentUsage);
  }

  private emitAssistantUpdate(event: AssistantMessageEvent): void {
    if (!this.assistantStarted) return;
    const partial = this.snapshot();
    // Replace the running snapshot in messages so renderers reading
    // `state.messages` see the same content as `assistantMessageEvent.partial`.
    this._state.messages[this._state.messages.length - 1] = partial;
    this._state.streamMessage = partial;
    this.emit({ type: 'message_update', message: partial, assistantMessageEvent: event });
  }

  private finalizeAssistantTurn(stopReason: 'stop' | 'aborted' | 'error'): void {
    if (!this.turnActive) return;
    // If the assistant never produced any content, synthesize an empty
    // message so the lifecycle stays well-formed (mirrors pi-agent-core
    // appending the final message on `done`).
    if (!this.assistantStarted) {
      const empty = this.snapshot();
      empty.stopReason = stopReason;
      this._state.messages.push(empty);
      this.emit({ type: 'message_start', message: empty });
      this.emit({ type: 'message_end', message: empty });
      this.emitTurnEnd(empty);
      return;
    }
    const finalMsg = this.snapshot();
    finalMsg.stopReason = stopReason;
    this._state.messages[this._state.messages.length - 1] = finalMsg;
    this._state.streamMessage = null;
    this.emit({ type: 'message_end', message: finalMsg });

    // Append toolResult messages with their own message_start/end pairs
    // (mirrors `agent-loop.js:386-387`), then turn_end carries them.
    for (const result of this.pendingTurnToolResults) {
      this._state.messages.push(result);
      this.emit({ type: 'message_start', message: result });
      this.emit({ type: 'message_end', message: result });
    }
    this.emitTurnEnd(finalMsg);
  }

  private emitTurnEnd(message: AgentMessage): void {
    const toolResults = [...this.pendingTurnToolResults];
    this.pendingTurnToolResults.length = 0;
    this.emit({ type: 'turn_end', message, toolResults });

    this._state.isStreaming = false;
    this._state.streamMessage = null;
    this._state.pendingToolCalls = new Set();
    this.turnActive = false;
    this.assistantStarted = false;
    this.streamingContent = [];

    this.emit({ type: 'agent_end', messages: this._state.messages.slice() });
  }

  private terminateWithError(message: string, stopReason: 'error' | 'aborted' = 'error'): void {
    if (!this.turnActive) return;
    if (this.assistantStarted) {
      const errMsg = this.snapshot();
      errMsg.stopReason = stopReason;
      errMsg.errorMessage = message;
      this._state.messages[this._state.messages.length - 1] = errMsg;
      this._state.streamMessage = null;
      this.emit({ type: 'message_end', message: errMsg });
      this._state.error = message;
      this.emit({ type: 'turn_end', message: errMsg, toolResults: [] });
    } else {
      const errMsg = this.snapshot();
      errMsg.stopReason = stopReason;
      errMsg.errorMessage = message;
      this._state.messages.push(errMsg);
      this.emit({ type: 'message_start', message: errMsg });
      this.emit({ type: 'message_end', message: errMsg });
      this._state.error = message;
      this.emit({ type: 'turn_end', message: errMsg, toolResults: [] });
    }

    this._state.isStreaming = false;
    this._state.streamMessage = null;
    this._state.pendingToolCalls = new Set();
    this.turnActive = false;
    this.assistantStarted = false;
    this.streamingContent = [];
    this.pendingTurnToolResults.length = 0;

    this.emit({ type: 'agent_end', messages: this._state.messages.slice() });
  }

  private emit(event: RaraAgentEvent): void {
    for (const listener of this.listeners) {
      try {
        listener(event);
      } catch (err) {
        console.warn('RaraAgent: listener threw', err);
      }
    }
  }
}
