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

import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { SessionSearchHit } from '@/api/sessions';
import type { ChatSession } from '@/api/types';

const searchSessionsMock = vi.fn<(q: string, limit?: number) => Promise<SessionSearchHit[]>>();
vi.mock('@/api/sessions', async () => {
  const actual = await vi.importActual<typeof import('@/api/sessions')>('@/api/sessions');
  return {
    ...actual,
    searchSessions: (q: string, limit?: number) => searchSessionsMock(q, limit),
  };
});

const FIXED_ISO = '2025-06-15T12:00:00Z';

function sessionFixture(key: string, title: string): ChatSession {
  return {
    key,
    title,
    preview: '',
    updated_at: FIXED_ISO,
    created_at: FIXED_ISO,
    message_count: 1,
    model: null,
    model_provider: null,
    thinking_level: null,
    system_prompt: null,
    metadata: null,
  };
}

function hitFixture(overrides: Partial<SessionSearchHit> = {}): SessionSearchHit {
  return {
    session_key: 'hit-1',
    session_title: 'Hit Session',
    snippet: 'hello <mark>world</mark>',
    role: 'user',
    timestamp_ms: Date.UTC(2026, 3, 15, 12, 0, 0),
    seq: 3,
    ...overrides,
  };
}

describe('SessionSearchDialog', () => {
  beforeEach(() => {
    searchSessionsMock.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('renders recent sessions when the query is empty', async () => {
    const { SessionSearchDialog } = await import('../SessionSearchDialog');
    render(
      <SessionSearchDialog
        open
        onOpenChange={vi.fn()}
        onSelect={vi.fn()}
        recentSessions={[sessionFixture('a', 'Alpha'), sessionFixture('b', 'Beta')]}
      />,
    );
    expect(await screen.findByText('最近会话')).toBeInTheDocument();
    expect(screen.getByText('Alpha')).toBeInTheDocument();
    expect(screen.getByText('Beta')).toBeInTheDocument();
    expect(searchSessionsMock).not.toHaveBeenCalled();
  });

  it('debounces the search call', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    searchSessionsMock.mockResolvedValue([hitFixture()]);
    const { SessionSearchDialog } = await import('../SessionSearchDialog');

    render(
      <SessionSearchDialog open onOpenChange={vi.fn()} onSelect={vi.fn()} recentSessions={[]} />,
    );

    const input = await screen.findByPlaceholderText(/搜索会话/);
    act(() => {
      fireEvent.change(input, { target: { value: 'hello' } });
    });

    // Not fired yet — still inside the debounce window.
    expect(searchSessionsMock).not.toHaveBeenCalled();

    await act(async () => {
      vi.advanceTimersByTime(260);
    });

    expect(searchSessionsMock).toHaveBeenCalledTimes(1);
    expect(searchSessionsMock).toHaveBeenCalledWith('hello', 20);
    vi.useRealTimers();
  });

  it('renders the server snippet with <mark> highlights intact', async () => {
    searchSessionsMock.mockResolvedValue([hitFixture()]);
    const { SessionSearchDialog } = await import('../SessionSearchDialog');
    render(
      <SessionSearchDialog open onOpenChange={vi.fn()} onSelect={vi.fn()} recentSessions={[]} />,
    );
    const input = await screen.findByPlaceholderText(/搜索会话/);
    fireEvent.change(input, { target: { value: 'hello' } });
    await screen.findByText('Hit Session');
    // Dialog content lives in a portal, so query the document root.
    const mark = document.body.querySelector('mark');
    expect(mark).not.toBeNull();
    expect(mark?.textContent).toBe('world');
  });

  it('calls onSelect + onOpenChange(false) when a result is picked', async () => {
    searchSessionsMock.mockResolvedValue([hitFixture({ session_key: 'picked' })]);
    const onSelect = vi.fn();
    const onOpenChange = vi.fn();
    const { SessionSearchDialog } = await import('../SessionSearchDialog');
    render(
      <SessionSearchDialog
        open
        onOpenChange={onOpenChange}
        onSelect={onSelect}
        recentSessions={[]}
      />,
    );

    const input = await screen.findByPlaceholderText(/搜索会话/);
    fireEvent.change(input, { target: { value: 'hi' } });

    const row = await screen.findByText('Hit Session');
    fireEvent.click(row);

    await waitFor(() => expect(onSelect).toHaveBeenCalledWith('picked'));
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });
});
