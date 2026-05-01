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
 * BDD bindings for `specs/issue-2013-topology-timeline-history.spec.md`.
 *
 * Each `it(...)` name carries the spec's `Filter:` selector verbatim so
 * `agent-spec verify` can resolve scenarios to real test functions.
 */

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { TimelineView } from '../TimelineView';

import type { ChatMessageData } from '@/api/types';
import type { TopologyEventEntry } from '@/hooks/use-topology-subscription';

// --- Module mocks --------------------------------------------------------
//
// `TimelineView` reaches for: the persistent chat WS (`useChatSessionWs`),
// the chat-models cache, the new history fetch (`useSessionHistory`), and
// the vendored craft `InputContainer`. We replace them with thin stubs so
// the test exercises only the history-rendering wiring.

vi.mock('@/hooks/use-chat-session-ws', () => ({
  useChatSessionWs: () => ({
    status: 'live',
    error: null,
    sendPrompt: () => true,
    sendAbort: () => true,
  }),
}));

vi.mock('@/hooks/use-chat-models', () => ({
  useChatModels: () => ({ data: [] }),
}));

// Mock the underlying REST wrapper so `useSessionHistory` (a real
// react-query hook) drives the loading/success/error paths the spec
// requires.
const listMessagesMock = vi.fn();
vi.mock('@/api/sessions', () => ({
  listMessages: (...args: unknown[]) => listMessagesMock(...args),
}));

// `InputContainer` pulls in CodeMirror + tiptap which are heavy and
// irrelevant here. Render a lightweight placeholder with a stable
// data-testid so we can assert on its presence without booting the real
// editor.
vi.mock('~vendor/components/input/InputContainer', () => ({
  InputContainer: ({ disabled }: { disabled?: boolean }) => (
    <div data-testid="input-container" data-disabled={String(Boolean(disabled))}>
      input
    </div>
  ),
}));

vi.mock('~vendor/components/chat/UserMessageBubble', () => ({
  UserMessageBubble: ({ content }: { content: string }) => (
    <div data-testid="user-bubble">{content}</div>
  ),
}));

vi.mock('~vendor/context/AppShellContext', () => ({
  AppShellProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

vi.mock('~vendor/context/EscapeInterruptContext', () => ({
  EscapeInterruptProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

function makeMessage(partial: Partial<ChatMessageData>): ChatMessageData {
  return {
    seq: 0,
    role: 'user',
    content: '',
    created_at: '2026-04-30T00:00:00Z',
    ...partial,
  };
}

function renderTimeline(props: {
  viewSessionKey: string;
  events?: TopologyEventEntry[];
  promptSessionKey?: string | null;
}) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <TimelineView
        viewSessionKey={props.viewSessionKey}
        events={props.events ?? []}
        promptSessionKey={props.promptSessionKey ?? null}
      />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  listMessagesMock.mockReset();
});

afterEach(() => {
  cleanup();
});

describe('TimelineView.history', () => {
  it('renders_history_before_live_events: shows persisted user + assistant messages on mount', async () => {
    listMessagesMock.mockResolvedValueOnce([
      makeMessage({ seq: 1, role: 'user', content: 'hello' }),
      makeMessage({ seq: 2, role: 'assistant', content: 'hi there' }),
    ]);

    renderTimeline({ viewSessionKey: 'sess-A' });

    expect(await screen.findByText('hello')).toBeInTheDocument();
    expect(await screen.findByText('hi there')).toBeInTheDocument();
    expect(screen.queryByText(/Waiting for the next turn/i)).not.toBeInTheDocument();
  });

  it('session_switch_refetches: switching viewSessionKey resets the rendered timeline', async () => {
    listMessagesMock.mockImplementation((key: string) => {
      if (key === 'A') {
        return Promise.resolve([makeMessage({ seq: 1, role: 'assistant', content: 'from-A' })]);
      }
      return Promise.resolve([makeMessage({ seq: 1, role: 'assistant', content: 'from-B' })]);
    });

    const { rerender } = renderTimeline({ viewSessionKey: 'A' });

    expect(await screen.findByText('from-A')).toBeInTheDocument();

    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false, gcTime: 0 } },
    });
    rerender(
      <QueryClientProvider client={queryClient}>
        <TimelineView viewSessionKey="B" events={[]} promptSessionKey={null} />
      </QueryClientProvider>,
    );

    expect(await screen.findByText('from-B')).toBeInTheDocument();
    await waitFor(() => {
      expect(screen.queryByText('from-A')).not.toBeInTheDocument();
    });
  });

  it('boundary_dedupe: live events with seq <= last history seq render exactly once', async () => {
    listMessagesMock.mockResolvedValueOnce([
      makeMessage({ seq: 5, role: 'assistant', content: 'boundary-text' }),
    ]);

    const dupEvent: TopologyEventEntry = {
      seq: 5,
      sessionKey: 'sess-A',
      event: { type: 'text_delta', text: 'boundary-text' },
    };
    const doneEvent: TopologyEventEntry = {
      seq: 5,
      sessionKey: 'sess-A',
      event: { type: 'done' },
    };

    renderTimeline({ viewSessionKey: 'sess-A', events: [dupEvent, doneEvent] });

    await screen.findByText('boundary-text');

    // Exactly one DOM node carries the boundary text — the live duplicate
    // is filtered before the reducer ever sees it.
    const matches = screen.getAllByText('boundary-text');
    expect(matches).toHaveLength(1);
  });

  it('fetch_error_does_not_block_live: history failure surfaces inline error and keeps input working', async () => {
    listMessagesMock.mockRejectedValueOnce(new Error('HTTP 500'));

    renderTimeline({ viewSessionKey: 'sess-A' });

    expect(await screen.findByRole('alert')).toHaveTextContent(
      /Failed to load conversation history/i,
    );

    const input = screen.getByTestId('input-container');
    expect(input).toBeInTheDocument();
    // The input is disabled only when the WS status is `idle` or `closed`;
    // our mock returns `live`, so a history error must NOT disable it.
    expect(input).toHaveAttribute('data-disabled', 'false');
  });
});
