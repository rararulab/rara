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

import { describe, expect, it, vi } from 'vitest';

import { TapeReloadGate } from '../tape-reload-gate';

describe('TapeReloadGate', () => {
  it('reloads immediately when no chat WS turn is in flight', () => {
    const reload = vi.fn();
    const gate = new TapeReloadGate({ reload });

    gate.onTapeAppended();
    gate.onTapeAppended();

    expect(reload).toHaveBeenCalledTimes(2);
  });

  it('drops tape_appended events while a turn is in flight (chat WS owns content)', () => {
    const reload = vi.fn();
    const gate = new TapeReloadGate({ reload });

    gate.onStreamStarted();
    gate.onTapeAppended(); // user message persisted
    gate.onTapeAppended(); // assistant message persisted
    expect(reload).not.toHaveBeenCalled();
  });

  it('flushes one reload on stream close when events were queued', () => {
    const reload = vi.fn();
    const gate = new TapeReloadGate({ reload });

    gate.onStreamStarted();
    gate.onTapeAppended();
    gate.onTapeAppended();
    gate.onStreamClosed();

    expect(reload).toHaveBeenCalledTimes(1);
  });

  it('does not reload on stream close if no events were queued', () => {
    const reload = vi.fn();
    const gate = new TapeReloadGate({ reload });

    gate.onStreamStarted();
    gate.onStreamClosed();

    expect(reload).not.toHaveBeenCalled();
  });

  it('reloads normally after a turn ends', () => {
    const reload = vi.fn();
    const gate = new TapeReloadGate({ reload });

    gate.onStreamStarted();
    gate.onStreamClosed();
    expect(reload).not.toHaveBeenCalled();

    // Background-task summary lands later — should reload as before.
    gate.onTapeAppended();
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it('treats a spurious close (no started) as a no-op', () => {
    const reload = vi.fn();
    const gate = new TapeReloadGate({ reload });

    gate.onStreamClosed();
    expect(reload).not.toHaveBeenCalled();

    gate.onTapeAppended();
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it('handles back-to-back turns independently', () => {
    const reload = vi.fn();
    const gate = new TapeReloadGate({ reload });

    // Turn 1 — events queued, one flush.
    gate.onStreamStarted();
    gate.onTapeAppended();
    gate.onStreamClosed();
    expect(reload).toHaveBeenCalledTimes(1);

    // Turn 2 — no events, no flush.
    gate.onStreamStarted();
    gate.onStreamClosed();
    expect(reload).toHaveBeenCalledTimes(1);

    // Turn 3 — events queued again, second flush.
    gate.onStreamStarted();
    gate.onTapeAppended();
    gate.onStreamClosed();
    expect(reload).toHaveBeenCalledTimes(2);
  });
});
