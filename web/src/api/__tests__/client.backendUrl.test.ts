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

import { getBackendUrl } from '../client';

function installLocalStorageStub() {
  const store = new Map<string, string>();
  const stub = {
    getItem: (key: string) => store.get(key) ?? null,
    setItem: (key: string, value: string) => store.set(key, String(value)),
    removeItem: (key: string) => store.delete(key),
    clear: () => store.clear(),
    key: (index: number) => Array.from(store.keys())[index] ?? null,
    get length() {
      return store.size;
    },
  };
  vi.stubGlobal('localStorage', stub);
  Object.defineProperty(window, 'localStorage', { value: stub, configurable: true });
}

describe('getBackendUrl', () => {
  beforeEach(() => {
    installLocalStorageStub();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('defaults to the page origin when no explicit backend is configured', () => {
    expect(getBackendUrl()).toBe(window.location.origin);
  });

  it('preserves an HTTPS production origin', () => {
    vi.stubGlobal('window', { location: { origin: 'https://rara.crownni.com' } });

    expect(getBackendUrl()).toBe('https://rara.crownni.com');
  });
});
