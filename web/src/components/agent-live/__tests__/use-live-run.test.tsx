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

import { renderHook } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { EMPTY_SLICE, liveRunStore } from '../live-run-store';
import { useLiveRun } from '../use-live-run';

describe('useLiveRun', () => {
  it('returns the shared frozen EMPTY_SLICE when sessionKey is undefined', () => {
    // Guards against React error #185 (infinite re-render) — the hook
    // MUST return a referentially stable snapshot across calls when
    // there is no session to subscribe to. Rerendering the hook N times
    // must observe the same reference.
    const { result, rerender } = renderHook(({ sk }) => useLiveRun(sk), {
      initialProps: { sk: undefined as string | undefined },
    });
    const first = result.current;
    rerender({ sk: undefined });
    rerender({ sk: undefined });
    expect(result.current).toBe(first);
    expect(result.current).toBe(EMPTY_SLICE);
  });

  it('returns the same snapshot reference across renders when the slice is empty', () => {
    // Same guarantee but for a real session that has never received a
    // frame — `liveRunStore.snapshot` returns `EMPTY_SLICE` as the
    // fallback so every read should hit the same identity.
    const { result, rerender } = renderHook(() => useLiveRun('session-empty'));
    const first = result.current;
    rerender();
    rerender();
    expect(result.current).toBe(first);
    // And without having published anything, it is still the shared slice.
    liveRunStore.reset('session-empty');
    rerender();
    expect(result.current).toBe(EMPTY_SLICE);
  });
});
