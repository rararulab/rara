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
 * Decides when a `tape_appended` server event should trigger a full
 * `reloadMessages()` of the active session.
 *
 * The chat WebSocket (`createRaraStreamFn`) is the source of truth while
 * a turn is in flight: it streams the assistant's text directly into
 * pi-agent-core's message buffer. If `tape_appended` fires during that
 * window — which happens for the user's own message as soon as the
 * kernel persists it, and again for the assistant message right after
 * `done` — and the React layer reacts by calling
 * `agent.replaceMessages` + `reconstructFromMessages`, the in-flight WS
 * content is clobbered. The user sees a blank turn even though the
 * backend has the full reply persisted (#1877, #1923).
 *
 * Driving the gate from `agent.state.isStreaming` was insufficient
 * because pi-agent-core only flips `isStreaming = true` on the first
 * stream event, leaving a race window between submit and first event.
 * The chat WS adapter, by contrast, emits explicit lifecycle frames
 * (`__stream_started` / `__stream_closed` / `__stream_reconnect_failed`)
 * that bracket the entire turn.
 *
 * Rules:
 * 1. Outside an in-flight turn, every `tape_appended` reloads — that's
 *    the original `useSessionEvents` contract for background-task
 *    summaries.
 * 2. `tape_appended` events arriving while a turn is in flight are
 *    dropped: the chat WS already owns the rendered content.
 * 3. If at least one `tape_appended` fired during the in-flight window,
 *    a single reload is flushed when the turn closes. This catches
 *    background-task summaries that happened to land mid-turn.
 */
export interface TapeReloadGateCallbacks {
  /** Invoked when the gate decides a reload should happen. */
  reload: () => void;
}

export class TapeReloadGate {
  private inFlight = false;
  private pending = false;
  private readonly callbacks: TapeReloadGateCallbacks;

  constructor(callbacks: TapeReloadGateCallbacks) {
    this.callbacks = callbacks;
  }

  /** Mark the chat WS turn boundary open. Idempotent. */
  onStreamStarted(): void {
    this.inFlight = true;
  }

  /**
   * Mark the chat WS turn boundary closed. Flushes a single reload if
   * any `tape_appended` events were skipped during the window. Idempotent.
   */
  onStreamClosed(): void {
    if (!this.inFlight) {
      // Spurious close (e.g. duplicate lifecycle frame) — nothing to do.
      this.pending = false;
      return;
    }
    this.inFlight = false;
    if (this.pending) {
      this.pending = false;
      this.callbacks.reload();
    }
  }

  /**
   * Decide whether a `tape_appended` event should reload now. When a
   * turn is in flight, defers to {@link onStreamClosed}.
   */
  onTapeAppended(): void {
    if (this.inFlight) {
      this.pending = true;
      return;
    }
    this.callbacks.reload();
  }
}
