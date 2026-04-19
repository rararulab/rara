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

import { beforeEach, describe, expect, it, vi } from 'vitest';

// Stub the API module before importing the SUT so init()'s
// fetch-through seed doesn't fire real requests.
const apiGet = vi.fn();
const apiDel = vi.fn();
const settingsList = vi.fn();
const settingsSet = vi.fn();
const settingsDelete = vi.fn();

vi.mock('@/api/client', () => ({
  api: {
    get: (path: string) => apiGet(path),
    del: (path: string) => apiDel(path),
  },
  settingsApi: {
    list: () => settingsList(),
    set: (k: string, v: string) => settingsSet(k, v),
    delete: (k: string) => settingsDelete(k),
  },
}));

const { RaraStorageBackend } = await import('../rara-storage');

describe('RaraStorageBackend — session store invariants', () => {
  beforeEach(() => {
    apiGet.mockReset();
    apiDel.mockReset();
    settingsList.mockReset();
    settingsSet.mockReset();
    settingsDelete.mockReset();
    // init() reads sessions + settings; stub both with empty payloads.
    apiGet.mockResolvedValue([]);
    settingsList.mockResolvedValue({});
    apiDel.mockResolvedValue(undefined);
    settingsSet.mockResolvedValue(undefined);
  });

  it('writes a new session into both `sessions` and `sessions-metadata`', async () => {
    const store = new RaraStorageBackend();
    await store.init();

    const meta = { id: 's1', title: 'Hello', lastModified: '2025-01-01' };
    await store.set('sessions', 's1', meta);

    // Both stores mirror the write — pi-web-ui reads from either.
    expect(await store.get('sessions', 's1')).toEqual(meta);
    expect(await store.get('sessions-metadata', 's1')).toEqual(meta);
  });

  it('switches to a different session without disturbing the previous entry', async () => {
    const store = new RaraStorageBackend();
    await store.init();

    await store.set('sessions', 's1', { id: 's1', title: 'First' });
    await store.set('sessions', 's2', { id: 's2', title: 'Second' });

    expect(await store.get('sessions', 's1')).toEqual({ id: 's1', title: 'First' });
    expect(await store.get('sessions', 's2')).toEqual({ id: 's2', title: 'Second' });
  });

  it('deleting a session removes it from both stores and fires the backend DELETE', async () => {
    const store = new RaraStorageBackend();
    await store.init();

    await store.set('sessions', 's1', { id: 's1', title: 'Doomed' });
    await store.delete('sessions', 's1');

    expect(await store.get('sessions', 's1')).toBeNull();
    expect(await store.get('sessions-metadata', 's1')).toBeNull();
    expect(apiDel).toHaveBeenCalledWith('/api/v1/chat/sessions/s1');
  });

  it('writing to `sessions-metadata` also mirrors into `sessions`', async () => {
    const store = new RaraStorageBackend();
    await store.init();

    await store.set('sessions-metadata', 's1', { id: 's1', title: 'Meta-first' });
    expect(await store.get('sessions', 's1')).toEqual({ id: 's1', title: 'Meta-first' });
  });
});
