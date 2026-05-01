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

/**
 * BDD bindings for `specs/issue-2022-topology-collapsible-sidebar.spec.md`.
 *
 * Each `it(...)` name carries the spec's `Test:` selector verbatim so
 * `agent-spec verify` can resolve scenarios to real test functions.
 */

import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitForElementToBeRemoved,
} from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// Node 25's experimental built-in `localStorage` shim is enabled by
// default in jsdom but exposes a hollow `[Object: null prototype] {}` —
// `setItem` / `getItem` / `removeItem` are missing. Install a minimal
// in-memory `Storage`-shaped stand-in before importing the component so
// `readSidebarCollapsed()` and the persistence effect see a working
// surface. Running these tests under jsdom is fine; the shim only
// needs to support the methods this page calls.
class MemoryStorage {
  private store = new Map<string, string>();
  getItem(key: string): string | null {
    return this.store.has(key) ? (this.store.get(key) ?? null) : null;
  }
  setItem(key: string, value: string): void {
    this.store.set(key, String(value));
  }
  removeItem(key: string): void {
    this.store.delete(key);
  }
  clear(): void {
    this.store.clear();
  }
  key(index: number): string | null {
    return Array.from(this.store.keys())[index] ?? null;
  }
  get length(): number {
    return this.store.size;
  }
}
Object.defineProperty(window, 'localStorage', {
  configurable: true,
  value: new MemoryStorage(),
});

import Chat from '../Chat';

// --- Module mocks --------------------------------------------------------
//
// `Chat` mounts `SessionPicker`, `TimelineView`, `WorkerInbox`,
// `TapeLineageView`, and the topology WS subscription. The toggle test
// exercises the layout/grid level only — children are stubbed to thin
// markers so the assertions can target the picker's presence in the DOM
// without spinning up react-query, the WS, or the editor.

vi.mock('@/components/topology/SessionPicker', () => ({
  SessionPicker: () => <div data-testid="session-picker">picker</div>,
}));

vi.mock('@/components/topology/TimelineView', () => ({
  TimelineView: () => <div data-testid="timeline-view">timeline</div>,
}));

vi.mock('@/components/topology/WorkerInbox', () => ({
  WorkerInbox: () => <div data-testid="worker-inbox">workers</div>,
}));

vi.mock('@/components/topology/TapeLineageView', () => ({
  TapeLineageView: () => <div data-testid="tape-lineage">lineage</div>,
}));

// Issue #2040 added a chapter strip + a `useQuery` for the session row so
// the strip can read `anchors[]`. The sidebar tests don't care about the
// chapter UI, so stub the strip and the query+helper to keep this test
// focused on the layout/persistence behaviour it was written for.
vi.mock('@/components/topology/TimelineChapterStrip', () => ({
  TimelineChapterStrip: () => <div data-testid="chapter-strip" />,
}));
vi.mock('@/api/sessions', async () => {
  const actual = await vi.importActual<typeof import('@/api/sessions')>('@/api/sessions');
  return {
    ...actual,
    fetchSessionMessagesBetweenAnchors: vi.fn(async () => []),
  };
});
vi.mock('@tanstack/react-query', async () => {
  const actual =
    await vi.importActual<typeof import('@tanstack/react-query')>('@tanstack/react-query');
  return {
    ...actual,
    useQuery: () => ({ data: null, isLoading: false, isError: false, isSuccess: true }),
  };
});

vi.mock('@/hooks/use-topology-subscription', () => ({
  useTopologySubscription: () => ({
    status: { kind: 'idle' as const },
    events: [],
  }),
}));

// `react-router`'s `useParams` / `useNavigate` work without a Router only
// in v7 when not used through context; provide stable stubs to avoid the
// "useNavigate may be used only in the context of a Router" runtime error.
vi.mock('react-router', () => ({
  useParams: () => ({}),
  useNavigate: () => () => {},
}));

const STORAGE_KEY = 'rara.topology.sidebarCollapsed';

beforeEach(() => {
  window.localStorage.removeItem(STORAGE_KEY);
});

afterEach(() => {
  cleanup();
  window.localStorage.removeItem(STORAGE_KEY);
  vi.restoreAllMocks();
});

describe('Chat — collapsible sidebar (issue-2022)', () => {
  it('toggle_hides_session_picker', async () => {
    render(<Chat />);

    // Default expanded: picker is in the DOM.
    expect(screen.getByTestId('session-picker')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Hide sidebar' }));

    // After toggle: SessionPicker is removed from the DOM. The
    // `<aside>` wrapper is wrapped in `AnimatePresence` (issue-2042
    // polish), so use `waitForElementToBeRemoved` to give the exit
    // animation a tick to complete before asserting absence.
    await waitForElementToBeRemoved(() => screen.queryByTestId('session-picker'));
    // Toggle button now reflects the collapsed state.
    expect(screen.getByRole('button', { name: 'Show sidebar' })).toBeInTheDocument();
  });

  it('toggle_restores_session_picker', () => {
    window.localStorage.setItem(STORAGE_KEY, 'true');

    render(<Chat />);

    expect(screen.queryByTestId('session-picker')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Show sidebar' }));

    expect(screen.getByTestId('session-picker')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Hide sidebar' })).toBeInTheDocument();
  });

  it('collapsed_state_persists', () => {
    const { unmount } = render(<Chat />);

    fireEvent.click(screen.getByRole('button', { name: 'Hide sidebar' }));

    expect(window.localStorage.getItem(STORAGE_KEY)).toBe('true');

    unmount();
    cleanup();

    // Simulate a page reload backed by the same localStorage.
    render(<Chat />);

    expect(screen.queryByTestId('session-picker')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Show sidebar' })).toBeInTheDocument();
  });

  it('default_state_is_expanded', () => {
    expect(window.localStorage.getItem(STORAGE_KEY)).toBeNull();

    render(<Chat />);

    expect(screen.getByTestId('session-picker')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Hide sidebar' })).toBeInTheDocument();
  });

  it('localstorage_failure_falls_back', () => {
    // Force `getItem` to throw to simulate private browsing / disabled
    // storage. The component must fall back to the default expanded
    // state without surfacing the error.
    const getItemSpy = vi.spyOn(window.localStorage, 'getItem').mockImplementation(() => {
      throw new Error('storage disabled');
    });

    expect(() => render(<Chat />)).not.toThrow();

    expect(screen.getByTestId('session-picker')).toBeInTheDocument();

    getItemSpy.mockRestore();
  });
});
