/*
 * Copyright 2026 Rararulab
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

import { dataFeedsApi } from '../data-feeds';

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

describe('dataFeedsApi.events URL shape', () => {
  const originalFetch = globalThis.fetch;
  const calls: string[] = [];

  beforeEach(() => {
    installLocalStorageStub();
    calls.length = 0;
    globalThis.fetch = vi.fn(async (input: RequestInfo | URL) => {
      calls.push(typeof input === 'string' ? input : input.toString());
      return new Response(JSON.stringify({ events: [], total: 0, has_more: false }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    });
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    vi.unstubAllGlobals();
  });

  it('emits backend event_kinds filter while preserving pagination parameters', async () => {
    await dataFeedsApi.events('feed-1', {
      since: '24h',
      event_kinds: ['market_candle_closed', 'rss_article'],
      limit: 25,
      offset: 50,
    });

    expect(calls).toHaveLength(1);
    const url = calls[0]!;
    expect(url).toContain('/api/v1/data-feeds/feed-1/events');
    expect(url).toContain('since=24h');
    expect(url).toContain('event_kinds=market_candle_closed%2Crss_article');
    expect(url).toContain('limit=25');
    expect(url).toContain('offset=50');
  });
});
