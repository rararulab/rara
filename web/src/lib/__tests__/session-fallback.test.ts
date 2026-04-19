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

import { describe, expect, it } from 'vitest';

import { decidePostDeleteAction, pickSessionFallback } from '../session-fallback';

const session = (key: string) => ({ key });

describe('pickSessionFallback', () => {
  it('prefers the next neighbour when the middle row is deleted', () => {
    const sessions = [session('a'), session('b'), session('c')];
    expect(pickSessionFallback(sessions, 'b')).toEqual(session('c'));
  });

  it('falls back to the previous neighbour when the tail is deleted', () => {
    const sessions = [session('a'), session('b'), session('c')];
    expect(pickSessionFallback(sessions, 'c')).toEqual(session('b'));
  });

  it('returns null when the list becomes empty', () => {
    expect(pickSessionFallback([session('a')], 'a')).toBeNull();
  });

  it('returns null when the key is not in the list', () => {
    expect(pickSessionFallback([session('a'), session('b')], 'zzz')).toBeNull();
  });
});

describe('decidePostDeleteAction', () => {
  it('noops when an unrelated row is deleted', () => {
    const action = decidePostDeleteAction({
      activeSessionKey: 'a',
      deletedKey: 'b',
      fallback: session('c'),
    });
    expect(action).toEqual({ kind: 'noop' });
  });

  // Regression: deleting the currently-active session while other
  // sessions still exist MUST switch to a neighbour rather than
  // create a new one. Prior to #1609 the parent always spawned a
  // fresh session, which — combined with the sidebar's list refresh
  // — could trap the user in an infinite loop of empty sessions.
  it('switches to the sidebar-provided fallback rather than creating a new session', () => {
    const fallback = session('neighbour');
    const action = decidePostDeleteAction({
      activeSessionKey: 'active',
      deletedKey: 'active',
      fallback,
    });
    expect(action).toEqual({ kind: 'switch', session: fallback });
  });

  it('asks for a new session only when nothing is left', () => {
    const action = decidePostDeleteAction({
      activeSessionKey: 'solo',
      deletedKey: 'solo',
      fallback: null,
    });
    expect(action).toEqual({ kind: 'create-new' });
  });
});
