/*
 * Copyright 2025 Rararulab
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 */

/**
 * URL-shape tests for the new `fetchSessionMessagesBetweenAnchors`
 * helper (issue #2040). The contract under test is that the helper
 * passes `from_anchor` / `to_anchor` query params iff the caller
 * supplied a non-null id — the "most recent anchor" case
 * (`toAnchorId: null`) MUST NOT emit a `to_anchor=` param at all so the
 * backend reads to EOF.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { fetchSessionMessagesBetweenAnchors } from '../sessions';

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

describe('fetchSessionMessagesBetweenAnchors URL shape', () => {
  const originalFetch = globalThis.fetch;
  const calls: string[] = [];

  beforeEach(() => {
    installLocalStorageStub();
    calls.length = 0;
    globalThis.fetch = vi.fn(async (input: RequestInfo | URL) => {
      calls.push(typeof input === 'string' ? input : input.toString());
      return new Response('[]', {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    });
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    vi.unstubAllGlobals();
  });

  it('emits both from_anchor and to_anchor when both ids are set', async () => {
    await fetchSessionMessagesBetweenAnchors('abc', 7, 11);
    expect(calls).toHaveLength(1);
    const url = calls[0]!;
    expect(url).toContain('/api/v1/chat/sessions/abc/messages');
    expect(url).toContain('from_anchor=7');
    expect(url).toContain('to_anchor=11');
  });

  it('omits to_anchor when toAnchorId is null (most-recent case)', async () => {
    await fetchSessionMessagesBetweenAnchors('abc', 7, null);
    expect(calls).toHaveLength(1);
    const url = calls[0]!;
    expect(url).toContain('from_anchor=7');
    expect(url).not.toContain('to_anchor=');
  });

  it('omits from_anchor when fromAnchorId is null', async () => {
    await fetchSessionMessagesBetweenAnchors('abc', null, 11);
    expect(calls).toHaveLength(1);
    const url = calls[0]!;
    expect(url).toContain('to_anchor=11');
    expect(url).not.toContain('from_anchor=');
  });
});
