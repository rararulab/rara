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

import { render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { LiveRun, SessionSlice } from '../agent-live/live-run-store';

import type { ChatSession } from '@/api/types';

// Pinned so `formatRelativeDate` output stays stable across day/hour
// boundaries — otherwise CI occasionally flips between e.g. "1 hour
// ago" and "2 hours ago" and flakes the snapshot.
const FIXED_ISO = '2025-06-15T12:00:00Z';

// Minimal stub sessions — enough fields to render the history list.
const sessionFixture = (key: string, title: string): ChatSession => ({
  key,
  title,
  preview: '',
  updated_at: FIXED_ISO,
  created_at: FIXED_ISO,
  message_count: 1,
  model_provider: null,
  model: null,
  thinking_level: null,
  system_prompt: null,
  metadata: null,
});

// api.get is called by ChatSidebar inside useEffect; mock the module
// so the component renders a deterministic list without hitting the
// network.
const apiGet = vi.fn();
const apiDel = vi.fn();
vi.mock('@/api/client', () => ({
  api: {
    get: (path: string) => apiGet(path),
    del: (path: string) => apiDel(path),
  },
}));

// Hoist-safe mock of useLiveRun so SidebarRunHistory (rendered by
// ChatSidebar) can be fed synthetic run-history slices per test.
const useLiveRunMock = vi.fn<(key: string | undefined) => SessionSlice>();
vi.mock('../agent-live/use-live-run', () => ({
  useLiveRun: (key: string | undefined) => useLiveRunMock(key),
}));

const emptySlice: SessionSlice = { active: null, history: [] };

function runFixture(overrides: Partial<LiveRun> = {}): LiveRun {
  return {
    runId: 'r1',
    sessionKey: 'b',
    status: 'completed',
    startedAt: Date.UTC(2026, 3, 1, 12, 0, 0),
    endedAt: Date.UTC(2026, 3, 1, 12, 0, 5),
    items: [{ seq: 0, turn: 0, kind: 'tool_use', tool: 'Grep', input: { query: 'hello' } }],
    toolCalls: 1,
    error: null,
    currentStage: null,
    ...overrides,
  };
}

describe('ChatSidebar', () => {
  beforeEach(() => {
    apiGet.mockReset();
    apiDel.mockReset();
    useLiveRunMock.mockReset();
    useLiveRunMock.mockReturnValue(emptySlice);
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it('highlights the row whose key matches activeSessionKey', async () => {
    const sessions = [sessionFixture('a', 'Alpha'), sessionFixture('b', 'Beta')];
    apiGet.mockResolvedValueOnce(sessions);

    const { ChatSidebar } = await import('../ChatSidebar');

    render(
      <ChatSidebar
        activeSessionKey="b"
        onSelect={vi.fn()}
        onNewSession={vi.fn()}
        onOpenSettings={vi.fn()}
        onDeleteSession={vi.fn()}
        refreshKey={0}
      />,
    );

    // Wait for the async list to load.
    await waitFor(() => expect(screen.getByText('Beta')).toBeInTheDocument());

    // Each session row renders inside a group container; walk up to the
    // outer `group` div which carries the active-highlight class.
    const alphaRow = screen.getByText('Alpha').closest('.group');
    const betaRow = screen.getByText('Beta').closest('.group');

    // The active row carries the selected surface color; the inactive
    // row keeps the hover-only style. We assert on the distinctive
    // `bg-secondary/70` selector the component applies exclusively to
    // the active row.
    expect(betaRow?.className).toContain('bg-secondary/70');
    expect(alphaRow?.className).not.toContain('bg-secondary/70');
  });

  it('does not highlight any row when activeSessionKey is undefined', async () => {
    const sessions = [sessionFixture('a', 'Alpha')];
    apiGet.mockResolvedValueOnce(sessions);

    const { ChatSidebar } = await import('../ChatSidebar');

    render(
      <ChatSidebar
        activeSessionKey={undefined}
        onSelect={vi.fn()}
        onNewSession={vi.fn()}
        onOpenSettings={vi.fn()}
        onDeleteSession={vi.fn()}
        refreshKey={0}
      />,
    );

    await waitFor(() => expect(screen.getByText('Alpha')).toBeInTheDocument());
    const row = screen.getByText('Alpha').closest('.group');
    expect(row?.className).not.toContain('bg-secondary/70');
  });

  it('renders execution history section when the active session has runs', async () => {
    apiGet.mockResolvedValueOnce([sessionFixture('b', 'Beta')]);
    useLiveRunMock.mockReturnValue({
      active: null,
      history: [runFixture()],
    });

    const { ChatSidebar } = await import('../ChatSidebar');

    render(
      <ChatSidebar
        activeSessionKey="b"
        onSelect={vi.fn()}
        onNewSession={vi.fn()}
        onOpenSettings={vi.fn()}
        onDeleteSession={vi.fn()}
        refreshKey={0}
      />,
    );

    await waitFor(() => expect(screen.getByText('Beta')).toBeInTheDocument());
    expect(screen.getByText(/执行历史/)).toBeInTheDocument();
    // TaskRunHistory header exposes an accessible button with the label
    // "Execution history" — its presence confirms the sidebar rendered
    // the completed-run list, not merely the wrapper shell.
    expect(screen.getByText('Execution history')).toBeInTheDocument();
  });

  it('hides execution history section when there are no runs', async () => {
    apiGet.mockResolvedValueOnce([sessionFixture('b', 'Beta')]);
    useLiveRunMock.mockReturnValue(emptySlice);

    const { ChatSidebar } = await import('../ChatSidebar');

    render(
      <ChatSidebar
        activeSessionKey="b"
        onSelect={vi.fn()}
        onNewSession={vi.fn()}
        onOpenSettings={vi.fn()}
        onDeleteSession={vi.fn()}
        refreshKey={0}
      />,
    );

    await waitFor(() => expect(screen.getByText('Beta')).toBeInTheDocument());
    expect(screen.queryByText(/执行历史/)).not.toBeInTheDocument();
  });
});
