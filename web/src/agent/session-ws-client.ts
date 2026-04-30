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
 * Persistent per-session WebSocket client.
 *
 * Phase (c) of #1935. Owns one socket lifecycle per `sessionKey`, parses
 * the discriminated `WebFrame` union the backend `crates/channels::web_session`
 * emits, and exposes inbound `prompt` / `abort` builders. Reconnect uses
 * the same bounded exponential backoff schedule as the legacy
 * `rara-stream.ts` (mechanism constant — see anti-patterns guide).
 *
 * `Hello` is the only proof-of-life signal that resets the retry budget;
 * `onopen` alone is unreliable because the backend can early-close after
 * upgrade if the kernel handle is missing.
 */

import { buildWsBaseUrl } from '@/adapters/ws-base-url';
import { getAccessToken } from '@/api/client';

// ---------------------------------------------------------------------------
// Wire frames — server → client
//
// Mirrors `WebEvent` in `crates/channels/src/web.rs`. Keep in sync with
// the Rust enum: every variant the backend can send for the persistent
// `/api/v1/kernel/chat/session/{session_key}` endpoint must exist here.
// Unknown variants are tolerated by the parser (`type` not matched →
// frame ignored) so additive backend changes do not break old clients.
// ---------------------------------------------------------------------------

/** Discriminated union of frames received from the persistent session WS. */
export type WebFrame =
  | { type: 'hello' }
  | { type: 'message'; content: string }
  | { type: 'typing' }
  | { type: 'phase'; phase: string }
  | {
      type: 'error';
      message: string;
      category?: string | null;
      upgrade_url?: string | null;
    }
  | { type: 'text_delta'; text: string }
  | { type: 'reasoning_delta'; text: string }
  | { type: 'text_clear' }
  | {
      type: 'tool_call_start';
      name: string;
      id: string;
      arguments: Record<string, unknown>;
    }
  | {
      type: 'tool_call_end';
      id: string;
      result_preview: string;
      success: boolean;
      error: string | null;
    }
  | { type: 'turn_rationale'; text: string }
  | { type: 'progress'; stage: string }
  | {
      type: 'turn_metrics';
      duration_ms: number;
      iterations: number;
      tool_calls: number;
      model: string;
    }
  | {
      type: 'usage';
      input: number;
      output: number;
      cache_read: number;
      cache_write: number;
      total_tokens: number;
      cost: number;
      model: string;
    }
  | {
      type: 'plan_created';
      goal: string;
      total_steps: number;
      compact_summary: string;
      estimated_duration_secs: number | null;
    }
  | {
      type: 'plan_progress';
      current_step: number;
      total_steps: number;
      step_status: unknown;
      status_text: string;
    }
  | { type: 'plan_replan'; reason: string }
  | { type: 'plan_completed'; summary: string }
  | {
      type: 'background_task_started';
      task_id: string;
      agent_name: string;
      description: string;
    }
  | { type: 'background_task_done'; task_id: string; status: unknown }
  | { type: 'trace_ready'; trace_id: string }
  | {
      type: 'attachment';
      tool_call_id: string | null;
      mime_type: string;
      filename: string | null;
      data_base64: string;
    }
  | {
      type: 'approval_requested';
      id: string;
      tool_name: string;
      summary: string;
      risk_level: string;
      requested_at: string;
      timeout_secs: number;
    }
  | { type: 'approval_resolved'; id: string; decision: string }
  | { type: 'done' }
  | {
      type: 'tape_appended';
      entry_id: number;
      role: string | null;
      timestamp: string;
    };

// ---------------------------------------------------------------------------
// Inbound frames — client → server
//
// Mirrors `InboundFrame` in `crates/channels/src/web_session.rs`.
// ---------------------------------------------------------------------------

/** Wire-format block sent inside `Prompt.content` for multimodal turns. */
export type PromptContentBlock =
  | { type: 'text'; text: string }
  | { type: 'image_base64'; media_type: string; data: string }
  | {
      // Server-side STT replaces `audio_base64` blocks with the
      // transcribed `text` block before the kernel sees them — see
      // `crates/channels/src/web.rs::transcribe_audio_blocks`. The
      // client only base64-encodes the recorded blob; transcription
      // never happens here.
      type: 'audio_base64';
      media_type: string;
      data: string;
    }
  | {
      type: 'file_base64';
      media_type: string;
      data: string;
      filename?: string;
    };

/** Inbound `prompt` payload — plain string OR multimodal block array. */
export type PromptContent = string | PromptContentBlock[];

/** Optional per-prompt overrides forwarded as top-level fields on the
 *  `prompt` frame. Currently just `model`, but kept as an options object
 *  so future per-turn knobs (thinking level, sampling) don't churn the
 *  signature. */
export interface PromptOptions {
  /** Pinned model id for this turn — mirrors the picker selection. The
   *  backend treats this as a session-sticky pin, matching the Telegram
   *  `/model` command path. */
  model?: string;
}

// ---------------------------------------------------------------------------
// Lifecycle events surfaced to RaraAgent
// ---------------------------------------------------------------------------

/**
 * Lifecycle notifications emitted independently of wire frames.
 *
 * - `connected`: a fresh socket received its `hello` frame. Subsequent
 *   `prompt` / `abort` calls will reach the backend.
 * - `reconnecting`: the previous socket dropped without a terminal frame
 *   and we are waiting `delayMs` before retry attempt `attempt` (1-based).
 * - `closed`: the client gave up — either consumer called `disconnect`,
 *   the retry budget is exhausted, or the backend rejected auth.
 */
export type LifecycleEvent =
  | { type: 'connected' }
  | { type: 'reconnecting'; attempt: number; delayMs: number }
  | { type: 'closed'; reason: 'user' | 'auth' | 'reconnect_exhausted' };

/**
 * Synthetic per-turn lifecycle frames synthesized by `RaraAgent` and
 * surfaced to the observer hook alongside raw `WebFrame`s.
 *
 * These mirror the legacy `rara-stream.ts` `__stream_*` frames so the
 * `live-run-store` reducer can keep working without semantic change:
 * a "stream" is now a single agentic turn rather than a per-turn
 * WebSocket lifecycle, but the timeline-card model is identical.
 *
 * - `__stream_started`: emitted right after `agent_start` for a new
 *   turn (i.e. when the user sends a prompt).
 * - `__stream_reconnecting`: forwarded from the underlying
 *   `LifecycleEvent` so the live card can show a grace-period state.
 * - `__stream_reconnect_failed`: emitted once the WS retry budget is
 *   exhausted; always immediately followed by `__stream_closed`.
 * - `__stream_closed`: emitted exactly once per turn when the turn
 *   reaches a terminal state (`done`/`error`/abort/reconnect_exhausted).
 */
export type StreamLifecycleEvent =
  | { type: '__stream_started' }
  | { type: '__stream_reconnecting'; attempt: number; delayMs: number }
  | { type: '__stream_reconnect_failed'; attempts: number }
  | { type: '__stream_closed' };

/**
 * Shape of events surfaced to the `RaraAgent` observer hook. The raw
 * WebSocket frame plus the four synthetic per-turn lifecycle frames.
 * Observers (e.g. the live-card store) can correlate
 * `tool_call_start`/`tool_call_end` pairs and distinguish run
 * boundaries via the synthetic frames.
 */
export type PublicWebEvent = WebFrame | StreamLifecycleEvent;

// ---------------------------------------------------------------------------
// Reconnect tuning — mechanism constants, NOT config
//
// Aligned with the legacy chat WS (`rara-stream.ts:118-120`) and the
// session-events WS (`use-session-events.ts:45-46`) so the new endpoint
// behaves the same as the two it replaces. See anti-patterns guide on
// "mechanism constants are not config".
// ---------------------------------------------------------------------------

const RECONNECT_BACKOFF_MS = [250, 500, 1_000, 2_000, 4_000] as const;
const RECONNECT_BACKOFF_CAP_MS = 5_000;
const MAX_RECONNECT_ATTEMPTS = RECONNECT_BACKOFF_MS.length;

// ---------------------------------------------------------------------------
// Public client surface
// ---------------------------------------------------------------------------

export interface SessionWsClientOptions {
  /** The session key to subscribe to. Frames flow from this session only. */
  sessionKey: string;
}

/** Frame handler. Exceptions are caught + logged so a buggy listener cannot tear down the socket. */
export type FrameHandler = (frame: WebFrame) => void;

/** Lifecycle handler. Same exception-safety as {@link FrameHandler}. */
export type LifecycleHandler = (event: LifecycleEvent) => void;

/**
 * Persistent WS client for `/api/v1/kernel/chat/session/{session_key}`.
 *
 * Owns one `WebSocket` and reconnects with bounded backoff on transport
 * drops. Does NOT translate frames into `RaraAgentEvent` — that's the
 * job of `RaraAgent`. This class stays a thin transport so it can be
 * tested in isolation.
 */
export class SessionWsClient {
  private readonly sessionKey: string;
  private socket: WebSocket | null = null;
  private reconnectAttempts = 0;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private disposed = false;
  private readonly frameHandlers = new Set<FrameHandler>();
  private readonly lifecycleHandlers = new Set<LifecycleHandler>();

  constructor(opts: SessionWsClientOptions) {
    this.sessionKey = opts.sessionKey;
  }

  /** Register a frame listener; returns an unsubscribe function. */
  onFrame(handler: FrameHandler): () => void {
    this.frameHandlers.add(handler);
    return () => {
      this.frameHandlers.delete(handler);
    };
  }

  /** Register a lifecycle listener; returns an unsubscribe function. */
  onLifecycle(handler: LifecycleHandler): () => void {
    this.lifecycleHandlers.add(handler);
    return () => {
      this.lifecycleHandlers.delete(handler);
    };
  }

  /** Open the socket. Idempotent — a second call while connecting is a no-op. */
  connect(): void {
    if (this.disposed) return;
    if (this.socket && this.socket.readyState <= 1) return;
    this.openSocket();
  }

  /**
   * Close the socket and stop reconnecting. After `disconnect()` the
   * client is terminal — construct a new instance to reconnect.
   */
  disconnect(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.clearReconnectTimer();
    this.detachAndClose();
    this.emitLifecycle({ type: 'closed', reason: 'user' });
  }

  /**
   * Send a `prompt` frame. The backend mirrors today's `submit_message`
   * path: transcribe audio, build the platform message, dispatch.
   *
   * Returns `false` if the socket is not open (caller can retry on
   * `connected`); in that case the prompt is dropped — no client-side
   * queueing, since the WS lifecycle is short and any pending frame
   * would be ambiguous after a session switch.
   */
  prompt(content: PromptContent, options: PromptOptions = {}): boolean {
    const payload: Record<string, unknown> = { type: 'prompt', content };
    if (options.model && options.model.length > 0) {
      payload.model = options.model;
    }
    return this.send(payload);
  }

  /** Send an `abort` frame. Same drop-on-disconnected semantics as `prompt`. */
  abort(): boolean {
    return this.send({ type: 'abort' });
  }

  // ------------------------------------------------------------------
  // Internals
  // ------------------------------------------------------------------

  private send(payload: Record<string, unknown>): boolean {
    if (!this.socket || this.socket.readyState !== 1) return false;
    try {
      this.socket.send(JSON.stringify(payload));
      return true;
    } catch (err) {
      console.warn('SessionWsClient: send failed', err);
      return false;
    }
  }

  private buildUrl(): string | null {
    const token = getAccessToken();
    // Token is optional in principle — the backend also reads
    // `Authorization` headers — but the browser WebSocket API does not
    // expose request headers, so the query fallback is mandatory in
    // practice. Without a token the backend will 401, so bail early.
    if (!token) return null;
    const base = buildWsBaseUrl();
    const path = `/api/v1/kernel/chat/session/${encodeURIComponent(this.sessionKey)}`;
    const params = new URLSearchParams({ token });
    return `${base}${path}?${params.toString()}`;
  }

  private openSocket(): void {
    const url = this.buildUrl();
    if (!url) {
      this.emitLifecycle({ type: 'closed', reason: 'auth' });
      this.disposed = true;
      return;
    }

    let ws: WebSocket;
    try {
      ws = new WebSocket(url);
    } catch (err) {
      console.warn('SessionWsClient: WebSocket constructor threw', err);
      this.scheduleReconnect();
      return;
    }
    this.socket = ws;

    ws.onmessage = (ev: MessageEvent) => {
      let frame: WebFrame;
      try {
        frame = JSON.parse(ev.data as string) as WebFrame;
      } catch {
        // Non-JSON frame — ignore. Backend never sends these but we
        // tolerate them so a stray binary frame doesn't crash the loop.
        return;
      }
      // `hello` is the only proof-of-life signal — backend can early-close
      // after `onopen` if the kernel handle is missing. Reset the retry
      // budget here, NOT in `onopen`.
      if (frame.type === 'hello') {
        this.reconnectAttempts = 0;
        this.emitLifecycle({ type: 'connected' });
      }
      this.emitFrame(frame);
    };

    // `onerror` is intentionally a no-op: browsers always fire `onclose`
    // after `onerror`, and routing reconnect through onclose keeps the
    // state machine single-sourced.
    ws.onerror = () => {};

    ws.onclose = () => {
      // Stale handler firing after a newer socket replaced this one.
      if (ws !== this.socket) return;
      this.socket = null;
      if (this.disposed) return;
      this.scheduleReconnect();
    };
  }

  private scheduleReconnect(): void {
    if (this.disposed) return;
    if (this.reconnectAttempts >= MAX_RECONNECT_ATTEMPTS) {
      this.disposed = true;
      this.emitLifecycle({ type: 'closed', reason: 'reconnect_exhausted' });
      return;
    }
    const delayMs = RECONNECT_BACKOFF_MS[this.reconnectAttempts] ?? RECONNECT_BACKOFF_CAP_MS;
    const attempt = this.reconnectAttempts + 1;
    this.reconnectAttempts = attempt;
    this.emitLifecycle({ type: 'reconnecting', attempt, delayMs });
    this.clearReconnectTimer();
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      if (this.disposed) return;
      this.openSocket();
    }, delayMs);
  }

  private clearReconnectTimer(): void {
    if (this.reconnectTimer !== null) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
  }

  private detachAndClose(): void {
    const ws = this.socket;
    if (!ws) return;
    this.socket = null;
    // Detach handlers first so an in-flight close does not schedule
    // another reconnect after the consumer asked us to stop.
    ws.onopen = null;
    ws.onmessage = null;
    ws.onerror = null;
    ws.onclose = null;
    try {
      ws.close();
    } catch {
      // Already closed; nothing to do.
    }
  }

  private emitFrame(frame: WebFrame): void {
    for (const handler of this.frameHandlers) {
      try {
        handler(frame);
      } catch (err) {
        console.warn('SessionWsClient: frame handler threw', err);
      }
    }
  }

  private emitLifecycle(event: LifecycleEvent): void {
    for (const handler of this.lifecycleHandlers) {
      try {
        handler(event);
      } catch (err) {
        console.warn('SessionWsClient: lifecycle handler threw', err);
      }
    }
  }
}
