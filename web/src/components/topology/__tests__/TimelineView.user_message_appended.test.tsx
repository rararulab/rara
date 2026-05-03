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
 * BDD bindings for the FE-bound scenarios in
 * `specs/issue-2063-user-message-appended-event.spec.md`.
 *
 * Each `it(...)` name carries the spec's `Filter:` suffix verbatim so
 * `agent-spec verify` can resolve scenarios to real test functions
 * (group prefix `TimelineView.user_message_appended` from the enclosing
 * `describe`).
 */

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { TimelineView } from '../TimelineView';

import type { ChatMessageData } from '@/api/types';
import type { TopologyEventEntry } from '@/hooks/use-topology-subscription';

// --- Module mocks --------------------------------------------------------

const chatWsMock = vi.hoisted(() => ({
  state: {
    status: 'live',
    error: null as string | null,
    sendPrompt: vi.fn(() => true),
    sendAbort: vi.fn(() => true),
  },
}));

vi.mock('@/hooks/use-chat-session-ws', () => ({
  useChatSessionWs: () => chatWsMock.state,
}));

vi.mock('@/hooks/use-chat-models', () => ({
  useChatModels: () => ({ data: [] }),
}));

const listMessagesMock = vi.fn();
vi.mock('@/api/sessions', () => ({
  listMessages: (...args: unknown[]) => listMessagesMock(...args),
}));

vi.mock('~vendor/components/input/InputContainer', () => ({
  InputContainer: ({
    onSubmit,
    disabled,
  }: {
    onSubmit?: (message: string) => void;
    disabled?: boolean;
  }) => (
    <div data-testid="input-container" data-disabled={String(Boolean(disabled))}>
      <button type="button" data-testid="input-submit" onClick={() => onSubmit?.('P')}>
        send
      </button>
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

function makeUserAppended(
  sessionKey: string,
  seq: number,
  text: string,
  createdAt = '2026-04-30T00:00:01Z',
  bufferSeq = seq,
): TopologyEventEntry {
  return {
    seq: bufferSeq,
    sessionKey,
    event: {
      type: 'user_message_appended',
      parent_session: sessionKey,
      seq,
      content: text,
      created_at: createdAt,
    },
  };
}

function renderTimeline(
  props: {
    viewSessionKey: string;
    events?: TopologyEventEntry[];
  },
  client?: QueryClient,
) {
  const queryClient =
    client ??
    new QueryClient({
      defaultOptions: { queries: { retry: false, gcTime: 0 } },
    });
  return {
    queryClient,
    ...render(
      <QueryClientProvider client={queryClient}>
        <TimelineView
          viewSessionKey={props.viewSessionKey}
          events={props.events ?? []}
          promptSessionKey={null}
        />
      </QueryClientProvider>,
    ),
  };
}

beforeEach(() => {
  listMessagesMock.mockReset();
  chatWsMock.state.status = 'live';
  chatWsMock.state.error = null;
  chatWsMock.state.sendPrompt.mockClear();
  chatWsMock.state.sendAbort.mockClear();
});

afterEach(() => {
  cleanup();
});

describe('TimelineView.user_message_appended', () => {
  it('no_duplicate_when_history_refetches_mid_turn: live frame + history refetch render exactly one bubble', async () => {
    // History initially returns []; after a refetch, returns the
    // persisted user message at the same seq the live frame carries.
    let resolveSecondHistory: (value: ChatMessageData[]) => void = () => {};
    listMessagesMock.mockResolvedValueOnce([]).mockReturnValueOnce(
      new Promise<ChatMessageData[]>((resolve) => {
        resolveSecondHistory = resolve;
      }),
    );

    const sessionKey = 'sess-A';
    const liveBubble = makeUserAppended(sessionKey, 1, 'P');
    // Render with the live frame already in the events buffer.
    // `useSessionHistory` resolves with `[]` so the barrier snapshots
    // at length 1 — the live bubble was already there at history-
    // resolution time. With the seq-dedupe in `orderedItems` (history
    // canonical), the post-refetch persisted bubble takes over without
    // duplicating.
    const { queryClient } = renderTimeline({
      viewSessionKey: sessionKey,
      events: [liveBubble],
    });

    await screen.findByText('P');
    expect(screen.getAllByText('P')).toHaveLength(1);

    // Mid-turn refetch (e.g. window focus). Resolve with the persisted
    // user message at seq=1 — same seq as the live frame. History wins
    // the seq-collision dedupe, so still exactly one bubble.
    void queryClient.refetchQueries({
      queryKey: ['topology', 'session-history', sessionKey],
    });
    await waitFor(() => expect(listMessagesMock).toHaveBeenCalledTimes(2));
    resolveSecondHistory([
      makeMessage({
        seq: 1,
        role: 'user',
        content: 'P',
        created_at: '2026-04-30T00:00:01Z',
      }),
    ]);

    await waitFor(() => {
      expect(screen.getAllByText('P')).toHaveLength(1);
    });
  });

  it('no_optimistic_state: submitting without any topology event yields zero bubbles', async () => {
    // The optimistic path is gone — submit through the input must NOT
    // produce a bubble locally; the bubble has to come from the
    // topology stream's user_message_appended frame.
    listMessagesMock.mockResolvedValueOnce([]);

    renderTimeline({ viewSessionKey: 'sess-no-opt', events: [] });

    // Wait for empty render.
    await screen.findByText(/Waiting for the next turn/i);

    // Click the input's send button (mocked to call onSubmit('P')).
    screen.getByTestId('input-submit').click();

    // Give React a tick.
    await new Promise((r) => setTimeout(r, 10));

    // No bubble should appear — the live frame would be the only source.
    expect(screen.queryByText('P')).not.toBeInTheDocument();
    // ws.sendPrompt was invoked (the path is otherwise functional).
    expect(chatWsMock.state.sendPrompt).toHaveBeenCalledWith('P', undefined);
  });

  it('bubble_from_topology_event_alone: empty history + live frame renders exactly one bubble keyed on seq', async () => {
    listMessagesMock.mockResolvedValueOnce([]);

    const sessionKey = 'sess-Q';
    const liveBubble = makeUserAppended(sessionKey, 1, 'Q', '2026-04-30T00:00:05Z');
    renderTimeline({
      viewSessionKey: sessionKey,
      events: [liveBubble],
    });

    const node = await screen.findByText('Q');
    expect(screen.getAllByText('Q')).toHaveLength(1);
    // Outer wrapper carries the React key derived from seq=1. RTL does
    // not expose the React `key` directly, but we can verify the render
    // is sourced from the live (`live-user-${seq}`) path by checking
    // it is a `user-bubble` node (via testid). The wrapper around it
    // has no DOM trace of the key, so the falsifier is "exactly one
    // node" combined with the bubble actually being present.
    expect(node).toHaveAttribute('data-testid', 'user-bubble');
  });

  it('reconnect_does_not_duplicate: WS reconnect + history refetch keeps exactly one bubble', async () => {
    const sessionKey = 'sess-R';

    // First history call: empty (so the barrier snapshots at 0 and the
    // live "R" bubble can flow through). Second call (post-reconnect
    // refetch): returns the persisted "R" at seq=1.
    let resolveSecond: (value: ChatMessageData[]) => void = () => {};
    listMessagesMock.mockResolvedValueOnce([]).mockReturnValueOnce(
      new Promise<ChatMessageData[]>((resolve) => {
        resolveSecond = resolve;
      }),
    );

    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false, gcTime: 0 } },
    });

    // Pre-reconnect: live R bubble + an in-flight assistant text_delta.
    const liveR = makeUserAppended(sessionKey, 1, 'R', '2026-04-30T00:00:01Z', 1);
    const liveDelta: TopologyEventEntry = {
      seq: 2,
      sessionKey,
      event: { type: 'text_delta', text: 'streaming-a' },
    };
    const { rerender } = render(
      <QueryClientProvider client={queryClient}>
        <TimelineView
          viewSessionKey={sessionKey}
          events={[liveR, liveDelta]}
          promptSessionKey={null}
        />
      </QueryClientProvider>,
    );

    await screen.findByText('R');
    expect(screen.getAllByText('R')).toHaveLength(1);
    await waitFor(() => expect(listMessagesMock).toHaveBeenCalledTimes(1));

    // Simulate WS reconnect: events buffer rebuilt from []. The
    // session-filtered length goes 2 → 0; the reset effect drops the
    // barrier and triggers history.refetch().
    rerender(
      <QueryClientProvider client={queryClient}>
        <TimelineView viewSessionKey={sessionKey} events={[]} promptSessionKey={null} />
      </QueryClientProvider>,
    );

    await waitFor(() => expect(listMessagesMock).toHaveBeenCalledTimes(2));

    // Refetch resolves with the persisted R.
    resolveSecond([
      makeMessage({
        seq: 1,
        role: 'user',
        content: 'R',
        created_at: '2026-04-30T00:00:01Z',
      }),
    ]);

    // Then a fresh user_message_appended for R seq=1 arrives on the
    // rebuilt buffer (kernel re-broadcasts on the new connection).
    const reLiveR = makeUserAppended(sessionKey, 1, 'R', '2026-04-30T00:00:01Z', 1);
    rerender(
      <QueryClientProvider client={queryClient}>
        <TimelineView viewSessionKey={sessionKey} events={[reLiveR]} promptSessionKey={null} />
      </QueryClientProvider>,
    );

    // Exactly one R — history takes precedence on seq collision so the
    // post-reconnect live frame is filtered out (seq dedupe in
    // orderedItems).
    await waitFor(() => {
      expect(screen.getAllByText('R')).toHaveLength(1);
    });
  });
});
