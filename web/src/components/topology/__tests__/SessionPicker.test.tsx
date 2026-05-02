/**
 * BDD bindings for `specs/issue-2043-session-status.spec.md`.
 *
 * Each `it(...)` carries the spec's `Filter:` selector verbatim so
 * `agent-spec verify` can resolve scenarios to real test functions.
 */

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { SessionPicker, SHOW_ARCHIVED_STORAGE_KEY } from '../SessionPicker';

import type { ChatSession } from '@/api/types';

// --- Mocks -----------------------------------------------------------------

const apiGetMock = vi.fn();
const apiPostMock = vi.fn();
const apiPatchMock = vi.fn();

vi.mock('@/api/client', () => ({
  api: {
    get: (...args: unknown[]) => apiGetMock(...args),
    post: (...args: unknown[]) => apiPostMock(...args),
    patch: (...args: unknown[]) => apiPatchMock(...args),
  },
}));

const updateSessionStatusMock = vi.fn();
vi.mock('@/api/sessions', async () => {
  const actual = await vi.importActual<typeof import('@/api/sessions')>('@/api/sessions');
  return {
    ...actual,
    updateSessionStatus: (...args: unknown[]) => updateSessionStatusMock(...args),
  };
});

// --- Fixture helpers -------------------------------------------------------

function makeSession(partial: Partial<ChatSession> & { key: string }): ChatSession {
  return {
    key: partial.key,
    title: partial.title ?? `session ${partial.key}`,
    model: null,
    model_provider: null,
    thinking_level: null,
    system_prompt: null,
    message_count: partial.message_count ?? 0,
    preview: null,
    metadata: null,
    created_at: partial.created_at ?? '2026-01-01T00:00:00Z',
    updated_at: partial.updated_at ?? '2026-01-01T00:00:00Z',
    status: partial.status ?? 'active',
  };
}

function buildClient() {
  return new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0, staleTime: 0 },
      mutations: { retry: false },
    },
  });
}

function renderPicker(activeSessionKey: string | null = null, onSelect = vi.fn()) {
  const client = buildClient();
  const utils = render(
    <QueryClientProvider client={client}>
      <SessionPicker activeSessionKey={activeSessionKey} onSelect={onSelect} />
    </QueryClientProvider>,
  );
  return { ...utils, client, onSelect };
}

// --- Setup -----------------------------------------------------------------

// Node 22+ ships a built-in `globalThis.localStorage` that shadows jsdom's
// implementation and lacks `setItem`/`getItem`/`clear` unless launched with
// `--localstorage-file`. Stub a minimal in-memory `Storage` so the toggle's
// `localStorage.getItem`/`setItem` round-trips against predictable state
// regardless of the host Node version. (Same pattern as
// `web/src/adapters/__tests__/ws-base-url.test.ts`.)
function installLocalStorageStub() {
  const store = new Map<string, string>();
  const stub = {
    getItem: (k: string) => (store.has(k) ? (store.get(k) ?? null) : null),
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

beforeEach(() => {
  apiGetMock.mockReset();
  apiPostMock.mockReset();
  apiPatchMock.mockReset();
  updateSessionStatusMock.mockReset();
  installLocalStorageStub();
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

// --- Spec scenarios --------------------------------------------------------

describe('SessionPicker', () => {
  it('SessionPicker hides archived rows by default', async () => {
    const fixture = [
      makeSession({ key: 'a', status: 'active', updated_at: '2026-01-03T00:00:00Z' }),
      makeSession({ key: 'b', status: 'active', updated_at: '2026-01-02T00:00:00Z' }),
      makeSession({ key: 'c', status: 'archived', updated_at: '2026-01-01T00:00:00Z' }),
    ];
    // Default view requests `?status=active`, so the backend would not
    // return the archived row. Simulate that here — the component must
    // not paper over a server filter by re-filtering client-side, but
    // it also must not surface the archived row when the API hands it
    // back, which is what we cover with the explicit fetch URL check.
    apiGetMock.mockImplementation((url: string) => {
      if (url.includes('status=active')) {
        return Promise.resolve(fixture.filter((s) => s.status === 'active'));
      }
      return Promise.resolve(fixture);
    });

    renderPicker();

    await waitFor(() => {
      expect(screen.getByText('session a')).toBeInTheDocument();
    });

    expect(screen.getByText('session a')).toBeInTheDocument();
    expect(screen.getByText('session b')).toBeInTheDocument();
    expect(screen.queryByText('session c')).not.toBeInTheDocument();

    // The component must request the active-only filter on the
    // initial fetch; otherwise the "default hides archived" guarantee
    // depends on every backend always defaulting too.
    expect(apiGetMock).toHaveBeenCalledWith(expect.stringContaining('status=active'));
  });

  it('SessionPicker show-archived toggle persists across remount', async () => {
    const fixture = [
      makeSession({ key: 'a', status: 'active', updated_at: '2026-01-03T00:00:00Z' }),
      makeSession({ key: 'b', status: 'active', updated_at: '2026-01-02T00:00:00Z' }),
      makeSession({ key: 'c', status: 'archived', updated_at: '2026-01-01T00:00:00Z' }),
    ];
    apiGetMock.mockImplementation((url: string) => {
      if (url.includes('status=active')) {
        return Promise.resolve(fixture.filter((s) => s.status === 'active'));
      }
      return Promise.resolve(fixture);
    });

    const first = renderPicker();
    await waitFor(() => expect(screen.getByText('session a')).toBeInTheDocument());

    // Click the toggle. After flip, fetch is rerun with status=all and
    // every row appears.
    const toggle = screen.getByTitle('Show archived');
    await act(async () => {
      fireEvent.click(toggle);
    });
    await waitFor(() => expect(screen.getByText('session c')).toBeInTheDocument());
    expect(screen.getAllByRole('listitem')).toHaveLength(3);
    expect(window.localStorage.getItem(SHOW_ARCHIVED_STORAGE_KEY)).toBe('true');

    // Remount: the persisted toggle keeps the archived row visible
    // without a click. Use a fresh QueryClient to mimic a real
    // navigation away and back — react-query state must not leak the
    // result.
    first.unmount();
    apiGetMock.mockClear();
    apiGetMock.mockImplementation((url: string) => {
      if (url.includes('status=active')) {
        return Promise.resolve(fixture.filter((s) => s.status === 'active'));
      }
      return Promise.resolve(fixture);
    });
    renderPicker();
    await waitFor(() => expect(screen.getByText('session c')).toBeInTheDocument());
    expect(apiGetMock).toHaveBeenCalledWith(expect.stringContaining('status=all'));
  });

  it('SessionPicker archive button removes row and disables on active', async () => {
    // Live fixture: the archive PATCH mutates this set so the next
    // refetch (triggered by `invalidateQueries`) returns the updated
    // list. Without that mutation the optimistic prune would be
    // overwritten by a refetch returning the unmodified fixture and
    // 'session b' would visibly bounce back into the rail.
    const sessions = new Map<string, ChatSession>([
      ['a', makeSession({ key: 'a', status: 'active', updated_at: '2026-01-03T00:00:00Z' })],
      ['b', makeSession({ key: 'b', status: 'active', updated_at: '2026-01-02T00:00:00Z' })],
      ['c', makeSession({ key: 'c', status: 'active', updated_at: '2026-01-01T00:00:00Z' })],
    ]);
    apiGetMock.mockImplementation((url: string) => {
      const wantsAll = url.includes('status=all');
      const list = Array.from(sessions.values());
      return Promise.resolve(wantsAll ? list : list.filter((s) => s.status === 'active'));
    });
    updateSessionStatusMock.mockImplementation((key: string, status: 'active' | 'archived') => {
      const prev = sessions.get(key);
      if (prev) sessions.set(key, { ...prev, status });
      return Promise.resolve({ ...makeSession({ key, status }) });
    });

    // None selected so all three get an enabled archive button at
    // first... but the spec explicitly says the archive button is
    // disabled on the active session. Using `activeSessionKey="a"`
    // gives us both the disabled-on-active and the
    // archive-removes-row assertions in one render.
    renderPicker('a');
    await waitFor(() => expect(screen.getByText('session a')).toBeInTheDocument());

    // Disabled tooltip on the active row.
    const activeArchiveBtn = screen.getByLabelText('Switch to another session first');
    expect(activeArchiveBtn).toBeDisabled();

    // Click archive on a non-active row. Use `getAllByLabelText` and
    // index by the button preceding 'session b' — the test asserts
    // structure, not pixel position.
    const archiveButtons = screen.getAllByLabelText('Archive session');
    expect(archiveButtons.length).toBeGreaterThan(0);
    const firstArchiveBtn = archiveButtons[0];
    if (!firstArchiveBtn) throw new Error('expected at least one Archive button');
    await act(async () => {
      fireEvent.click(firstArchiveBtn);
    });

    // The PATCH must carry exactly `{ status: 'archived' }`.
    await waitFor(() => {
      expect(updateSessionStatusMock).toHaveBeenCalledWith('b', 'archived');
    });

    // After the response resolves, the archived row drops out of the
    // default-active list — only `a` and `c` remain.
    await waitFor(() => {
      expect(screen.queryByText('session b')).not.toBeInTheDocument();
    });
    expect(screen.getByText('session a')).toBeInTheDocument();
    expect(screen.getByText('session c')).toBeInTheDocument();
  });
});
