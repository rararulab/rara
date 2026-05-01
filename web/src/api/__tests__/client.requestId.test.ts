/*
 * Copyright 2025 Rararulab
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { ApiError, api } from '../client';

// Node 22+ exposes a built-in `globalThis.localStorage` that shadows jsdom's
// implementation and lacks `getItem`/`setItem`. The api client's request
// helper reads the access token via `localStorage.getItem` before issuing
// the fetch, so we install a minimal in-memory Storage stub identical to
// `web/src/adapters/__tests__/ws-base-url.test.ts`.
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

/**
 * Spec scenario `api-client.requestId` from
 * `specs/issue-1975-trace-id-response-header.spec.md`: when a fetch returns
 * a non-2xx response with an `x-request-id` header, the thrown `ApiError`
 * exposes that value on its `requestId` field.
 */
describe('ApiError requestId propagation', () => {
  const fakeTraceId = '0123456789abcdef0123456789abcdef';
  const originalFetch = globalThis.fetch;

  beforeEach(() => {
    installLocalStorageStub();
    // Silence the console.error emitted by `raiseApiError` so test output
    // stays clean; the assertion is on the thrown ApiError, not the log.
    vi.spyOn(console, 'error').mockImplementation(() => {});
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('exposes the x-request-id header on the thrown ApiError', async () => {
    globalThis.fetch = vi.fn(
      async () =>
        new Response('boom', {
          status: 500,
          headers: { 'x-request-id': fakeTraceId },
        }),
    );

    let caught: unknown;
    try {
      await api.get('/api/v1/anything');
    } catch (err) {
      caught = err;
    }

    expect(caught).toBeInstanceOf(ApiError);
    const apiErr = caught as ApiError;
    expect(apiErr.status).toBe(500);
    expect(apiErr.requestId).toBe(fakeTraceId);
  });

  it('leaves requestId undefined when the response has no header', async () => {
    globalThis.fetch = vi.fn(async () => new Response('nope', { status: 500 }));

    let caught: unknown;
    try {
      await api.get('/api/v1/anything');
    } catch (err) {
      caught = err;
    }

    expect(caught).toBeInstanceOf(ApiError);
    expect((caught as ApiError).requestId).toBeUndefined();
  });
});
