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

const STORAGE_KEY = 'rara_backend_url';

// Node 22+ exposes a built-in `globalThis.localStorage` that shadows jsdom's
// implementation and lacks `setItem`/`getItem` unless launched with
// `--localstorage-file`. Tests here stub a minimal in-memory Storage so
// `buildWsBaseUrl`'s override probe runs against predictable state regardless
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

describe('buildWsBaseUrl — shared WS base URL resolver (#1921)', () => {
  beforeEach(() => {
    installLocalStorageStub();
    vi.resetModules();
  });

  afterEach(() => {
    localStorage.removeItem(STORAGE_KEY);
    vi.unstubAllGlobals();
    vi.doUnmock('@/api/client');
  });

  it('falls back to window.location when no override or BASE_URL is set', async () => {
    vi.doMock('@/api/client', () => ({
      BASE_URL: '',
      getBackendUrl: () => 'http://localhost:25555',
    }));
    const { buildWsBaseUrl } = await import('../ws-base-url');

    const loc = window.location;
    const proto = loc.protocol === 'https:' ? 'wss:' : 'ws:';
    expect(buildWsBaseUrl()).toBe(`${proto}//${loc.host}`);
  });

  it('honors rara_backend_url override (http -> ws)', async () => {
    vi.doMock('@/api/client', () => ({
      BASE_URL: '',
      getBackendUrl: () => 'http://10.0.0.183:25555',
    }));
    localStorage.setItem(STORAGE_KEY, 'http://10.0.0.183:25555');
    const { buildWsBaseUrl } = await import('../ws-base-url');

    expect(buildWsBaseUrl()).toBe('ws://10.0.0.183:25555');
  });

  it('strips a trailing slash from the override URL', async () => {
    vi.doMock('@/api/client', () => ({
      BASE_URL: '',
      getBackendUrl: () => 'https://backend.example.com/',
    }));
    localStorage.setItem(STORAGE_KEY, 'https://backend.example.com/');
    const { buildWsBaseUrl } = await import('../ws-base-url');

    expect(buildWsBaseUrl()).toBe('wss://backend.example.com');
  });

  it('honors compile-time BASE_URL when no override is set', async () => {
    vi.doMock('@/api/client', () => ({
      BASE_URL: 'https://prod.example.com',
      getBackendUrl: () => 'http://localhost:25555',
    }));
    const { buildWsBaseUrl } = await import('../ws-base-url');

    expect(buildWsBaseUrl()).toBe('wss://prod.example.com');
  });
});
