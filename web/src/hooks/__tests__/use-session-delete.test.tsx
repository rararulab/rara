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
import { describe, expect, it, vi } from 'vitest';

import { useSessionDelete } from '../use-session-delete';

interface Row {
  key: string;
}

describe('useSessionDelete', () => {
  it('switches to the fallback row when the active session is deleted', () => {
    const switchSession = vi.fn();
    const newSession = vi.fn();
    const fallback: Row = { key: 'neighbour' };

    const { result } = renderHook(() =>
      useSessionDelete<Row>({
        activeSessionKey: 'active',
        switchSession,
        newSession,
      }),
    );

    result.current('active', fallback);

    expect(switchSession).toHaveBeenCalledTimes(1);
    expect(switchSession).toHaveBeenCalledWith(fallback);
    expect(newSession).not.toHaveBeenCalled();
  });

  it('calls newSession exactly once when the last session is deleted (no infinite loop)', () => {
    const switchSession = vi.fn();
    const newSession = vi.fn();

    const { result } = renderHook(() =>
      useSessionDelete<Row>({
        activeSessionKey: 'solo',
        switchSession,
        newSession,
      }),
    );

    result.current('solo', null);

    expect(newSession).toHaveBeenCalledTimes(1);
    expect(switchSession).not.toHaveBeenCalled();
  });

  it('does nothing when an unrelated row is deleted', () => {
    const switchSession = vi.fn();
    const newSession = vi.fn();

    const { result } = renderHook(() =>
      useSessionDelete<Row>({
        activeSessionKey: 'active',
        switchSession,
        newSession,
      }),
    );

    result.current('some-other-row', { key: 'fallback-would-be-ignored' });

    expect(switchSession).not.toHaveBeenCalled();
    expect(newSession).not.toHaveBeenCalled();
  });

  it('does not fire newSession when a fallback is provided — guards the regression where deleting the active row re-spawned empty sessions', () => {
    const switchSession = vi.fn();
    const newSession = vi.fn();

    const { result } = renderHook(() =>
      useSessionDelete<Row>({
        activeSessionKey: 'active',
        switchSession,
        newSession,
      }),
    );

    // Call the handler the way the sidebar would: active row removed,
    // neighbour is still in the list.
    result.current('active', { key: 'neighbour' });

    expect(newSession).not.toHaveBeenCalled();
    expect(switchSession).toHaveBeenCalledWith({ key: 'neighbour' });
  });
});
