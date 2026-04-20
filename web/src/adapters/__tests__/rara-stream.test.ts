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

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { buildWsUrl } from '../rara-stream';

const STORAGE_KEY = 'rara_backend_url';

// Node 22+ exposes a built-in `globalThis.localStorage` that shadows jsdom's
// implementation and lacks `setItem`/`getItem` unless launched with
// `--localstorage-file`. Tests here stub a minimal in-memory Storage so
// `buildWsUrl`'s override probe runs against predictable state regardless
// of the host Node version.
function installLocalStorageStub() {
  const store = new Map<string, string>();
  const stub = {
    getItem: (k: string) => (store.has(k) ? store.get(k)! : null),
    setItem: (k: string, v: string) => {
      store.set(k, String(v));
    },
    removeItem: (k: string) => {
      store.delete(k);
    },
    clear: () => store.clear(),
    key: (i: number) => Array.from(store.keys())[i] ?? null,
    get length() {
      return store.size;
    },
  };
  vi.stubGlobal('localStorage', stub);
  Object.defineProperty(window, 'localStorage', { value: stub, configurable: true });
}

describe('buildWsUrl — backend override resolution (#1622)', () => {
  beforeEach(() => {
    installLocalStorageStub();
  });

  afterEach(() => {
    localStorage.removeItem(STORAGE_KEY);
    vi.unstubAllGlobals();
  });

  it('falls back to window.location when no override is set', () => {
    const url = buildWsUrl('sess-abc');
    const loc = window.location;
    const proto = loc.protocol === 'https:' ? 'wss:' : 'ws:';
    expect(url).toBe(
      `${proto}//${loc.host}/api/v1/kernel/chat/ws?session_key=sess-abc&user_id=web_ryan`,
    );
  });

  it('honors rara_backend_url override (http -> ws)', () => {
    localStorage.setItem(STORAGE_KEY, 'http://10.0.0.183:25555');
    expect(buildWsUrl('sess-abc')).toBe(
      'ws://10.0.0.183:25555/api/v1/kernel/chat/ws?session_key=sess-abc&user_id=web_ryan',
    );
  });

  it('honors rara_backend_url override with https and trims trailing slash', () => {
    localStorage.setItem(STORAGE_KEY, 'https://backend.example.com/');
    expect(buildWsUrl('sess-xyz')).toBe(
      'wss://backend.example.com/api/v1/kernel/chat/ws?session_key=sess-xyz&user_id=web_ryan',
    );
  });

  it('URL-encodes session keys containing special characters', () => {
    localStorage.setItem(STORAGE_KEY, 'http://10.0.0.183:25555');
    expect(buildWsUrl('sess/with spaces')).toBe(
      'ws://10.0.0.183:25555/api/v1/kernel/chat/ws?session_key=sess%2Fwith%20spaces&user_id=web_ryan',
    );
  });
});
