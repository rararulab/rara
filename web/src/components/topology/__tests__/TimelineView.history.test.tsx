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

  it('arrival_barrier_dedupe: live events that arrived before history resolved are not re-rendered after history loads', async () => {
    // Defer the history fetch so we can prove the live path renders
    // "boundary-text" BEFORE history settles, then resolve with a
    // ChatMessage that already covers the same delta. Spec scenario
    // `TimelineView.history.arrival_barrier_dedupe`.
    let resolveHistory: (value: ChatMessageData[]) => void = () => {};
    listMessagesMock.mockReturnValueOnce(
      new Promise<ChatMessageData[]>((resolve) => {
        resolveHistory = resolve;
      }),
    );

    // A real `text_delta` + `done` pair through the topology subscription
    // — drives the live reducer rather than a synthetic seq value.
    const liveDelta: TopologyEventEntry = {
      seq: 1,
      sessionKey: 'sess-A',
      event: { type: 'text_delta', text: 'boundary-text' },
    };
    const liveDone: TopologyEventEntry = {
      seq: 2,
      sessionKey: 'sess-A',
      event: { type: 'done' },
    };

    renderTimeline({ viewSessionKey: 'sess-A', events: [liveDelta, liveDone] });

    // Pre-history: live path is the only source of "boundary-text".
    await screen.findByText('boundary-text');
    expect(screen.getAllByText('boundary-text')).toHaveLength(1);

    // Resolve history with one assistant message whose content matches.
    // The arrival barrier should snapshot at `sessionEvents.length === 2`
    // and drop both pre-history live entries from the live reducer; the
    // history reducer then becomes the sole renderer.
    resolveHistory([makeMessage({ seq: 1, role: 'assistant', content: 'boundary-text' })]);

    await waitFor(() => {
      // Still exactly one — not duplicated by the live reducer.
      expect(screen.getAllByText('boundary-text')).toHaveLength(1);
    });
  });

  it('reconnect_resnapshots_barrier: WS reconnect re-snapshots the barrier even when history payload is structurally unchanged', async () => {
    // After a WS reconnect, `use-topology-subscription` rebuilds its
    // events buffer from `[]` on the `hello` frame. `TimelineView`
    // detects this via the session-filtered buffer length going
    // backwards, drops the stale barrier, and calls
    // `history.refetch()`. If the persisted history hasn't changed, the
    // refetched payload is structurally identical and react-query's
    // default `structuralSharing` returns the same array reference.
    // The barrier-snapshot effect must still re-run — otherwise live
    // events that arrived before the reconnect (and are now mirrored in
    // the rebuilt buffer) would render again on top of history.
    const persistedHistory: ChatMessageData[] = [
      makeMessage({ seq: 1, role: 'assistant', content: 'X' }),
    ];
    // Both calls (initial + post-reconnect refetch) return arrays with
    // identical contents — react-query's structural sharing will hand
    // out the same reference for both.
    listMessagesMock.mockResolvedValue(persistedHistory.map((m) => ({ ...m })));

    // Initial render: empty events buffer so the barrier snapshots at
    // length 0; history's "X" renders via the history path. Then push
    // a live `text_delta` carrying "Y" — index 0 is >= barrier so it
    // renders live.
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false, gcTime: 0 } },
    });
    const { rerender } = render(
      <QueryClientProvider client={queryClient}>
        <TimelineView viewSessionKey="S" events={[]} promptSessionKey={null} />
      </QueryClientProvider>,
    );

    await screen.findByText('X');
    expect(screen.getAllByText('X')).toHaveLength(1);

    const liveY: TopologyEventEntry = {
      seq: 1,
      sessionKey: 'S',
      event: { type: 'text_delta', text: 'Y' },
    };
    const liveYDone: TopologyEventEntry = {
      seq: 2,
      sessionKey: 'S',
      event: { type: 'done' },
    };
    rerender(
      <QueryClientProvider client={queryClient}>
        <TimelineView viewSessionKey="S" events={[liveY, liveYDone]} promptSessionKey={null} />
      </QueryClientProvider>,
    );
    await screen.findByText('Y');

    // Simulate WS reconnect: the topology subscription rebuilds its
    // buffer from `[]`, so the session-filtered length goes 2 → 0. The
    // reset effect fires, drops the stale barrier, and calls
    // `history.refetch()`. Because the persisted history is unchanged,
    // react-query's default `structuralSharing` returns the same array
    // reference. With the buggy dep array `[..., historyMessages, ...]`
    // the barrier-snapshot effect does not re-run, leaving the barrier
    // permanently deleted; `agentTurns` then falls back to the
    // `barrier === undefined` branch (render every sessionEvent), so a
    // post-reconnect live frame whose content matches an already-
    // persisted message renders on top of history.
    rerender(
      <QueryClientProvider client={queryClient}>
        <TimelineView viewSessionKey="S" events={[]} promptSessionKey={null} />
      </QueryClientProvider>,
    );

    const reLiveX: TopologyEventEntry = {
      seq: 1,
      sessionKey: 'S',
      event: { type: 'text_delta', text: 'X' },
    };
    const reLiveXDone: TopologyEventEntry = {
      seq: 2,
      sessionKey: 'S',
      event: { type: 'done' },
    };
    rerender(
      <QueryClientProvider client={queryClient}>
        <TimelineView viewSessionKey="S" events={[reLiveX, reLiveXDone]} promptSessionKey={null} />
      </QueryClientProvider>,
    );

    // Without the `dataUpdatedAt` dep fix, the barrier-snapshot effect
    // would not re-run after refetch (history reference unchanged), the
    // barrier would stay deleted, and the live path would render
    // `reLiveX` as a fresh "X" — duplicating history.
    await waitFor(() => {
      expect(screen.getAllByText('X')).toHaveLength(1);
    });
  });

  it('chronological_ordering_history_only: interleaves user bubbles and assistant turns by created_at', async () => {
    // Spec scenario `TimelineView.history.chronological_ordering_history_only`.
    listMessagesMock.mockResolvedValueOnce([
      makeMessage({
        seq: 1,
        role: 'user',
        content: 'q1',
        created_at: '2026-04-30T00:00:01Z',
      }),
      makeMessage({
        seq: 2,
        role: 'assistant',
        content: 'a1',
        created_at: '2026-04-30T00:00:02Z',
      }),
      makeMessage({
        seq: 3,
        role: 'user',
        content: 'q2',
        created_at: '2026-04-30T00:00:03Z',
      }),
      makeMessage({
        seq: 4,
        role: 'assistant',
        content: 'a2',
        created_at: '2026-04-30T00:00:04Z',
      }),
    ]);

    renderTimeline({ viewSessionKey: 'sess-A' });

    await screen.findByText('a2');
    const nodes = screen.getAllByTestId('turn-or-bubble');
    expect(nodes).toHaveLength(4);
    expect(nodes[0]).toHaveTextContent('q1');
    expect(nodes[1]).toHaveTextContent('a1');
    expect(nodes[2]).toHaveTextContent('q2');
    expect(nodes[3]).toHaveTextContent('a2');
  });

  it('chronological_ordering_history_then_live: live agent turn renders strictly after historical entries', async () => {
    // Spec scenario `TimelineView.history.chronological_ordering_history_then_live`.
    listMessagesMock.mockResolvedValueOnce([
      makeMessage({
        seq: 1,
        role: 'user',
        content: 'hist-q',
        created_at: '2026-04-30T00:00:01Z',
      }),
      makeMessage({
        seq: 2,
        role: 'assistant',
        content: 'hist-a',
        created_at: '2026-04-30T00:00:02Z',
      }),
    ]);

    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false, gcTime: 0 } },
    });
    const { rerender } = render(
      <QueryClientProvider client={queryClient}>
        <TimelineView viewSessionKey="sess-A" events={[]} promptSessionKey={null} />
      </QueryClientProvider>,
    );

    await screen.findByText('hist-a');

    const liveDelta: TopologyEventEntry = {
      seq: 1,
      sessionKey: 'sess-A',
      event: { type: 'text_delta', text: 'live-a' },
    };
    const liveDone: TopologyEventEntry = {
      seq: 2,
      sessionKey: 'sess-A',
      event: { type: 'done' },
    };
    rerender(
      <QueryClientProvider client={queryClient}>
        <TimelineView
          viewSessionKey="sess-A"
          events={[liveDelta, liveDone]}
          promptSessionKey={null}
        />
      </QueryClientProvider>,
    );

    await screen.findByText('live-a');
    const nodes = screen.getAllByTestId('turn-or-bubble');
    expect(nodes).toHaveLength(3);
    expect(nodes[0]).toHaveTextContent('hist-q');
    expect(nodes[1]).toHaveTextContent('hist-a');
    expect(nodes[2]).toHaveTextContent('live-a');
  });

  it('assistant_turn_height_matches_content: empty assistant tape entries do not produce blank turn cards', async () => {
    // Spec scenario `TimelineView.history.assistant_turn_height_matches_content`.
    // Reproduces the API-observed pattern: a real assistant message
    // followed by tool-call slot entries with empty content. Without the
    // fix, each empty entry would emit its own blank `Card`, producing
    // over-tall empty boxes in the rendered DOM.
    listMessagesMock.mockResolvedValueOnce([
      makeMessage({
        seq: 1,
        role: 'assistant',
        content: 'ok',
        created_at: '2026-04-30T00:00:01Z',
      }),
      makeMessage({
        seq: 2,
        role: 'user',
        content: 'q',
        created_at: '2026-04-30T00:00:02Z',
      }),
      // Tool-call slot entries the backend emits with empty content.
      // These must not produce blank turn cards.
      makeMessage({
        seq: 3,
        role: 'assistant',
        content: '',
        created_at: '2026-04-30T00:00:03Z',
      }),
      makeMessage({
        seq: 4,
        role: 'assistant',
        content: '\n',
        created_at: '2026-04-30T00:00:04Z',
      }),
      makeMessage({
        seq: 5,
        role: 'assistant',
        content: '',
        created_at: '2026-04-30T00:00:05Z',
      }),
    ]);

    renderTimeline({ viewSessionKey: 'sess-A' });

    await screen.findByText('ok');
    // 1 assistant turn ("ok") + 1 user bubble ("q"). The whitespace-only
    // assistant entries (seq 3, 4, 5) — which the backend persists as
    // tool-call slot remnants — must not contribute their own blank turn
    // cards or prepend leading newlines to a neighbouring turn.
    const nodes = screen.getAllByTestId('turn-or-bubble');
    expect(nodes).toHaveLength(2);
    expect(nodes[0]).toHaveTextContent('ok');
    expect(nodes[1]).toHaveTextContent('q');
    // The toHaveLength(2) above is the real falsifier here: under the
    // "unconditional min-height" failure mode, an extra empty card or a
    // newline-prepended neighbour would push the count off 2. (A direct
    // height assertion would be a no-op under jsdom, which returns 0
    // from getBoundingClientRect.)
    const card = nodes[0].querySelector('div.space-y-3.p-4');
    expect(card).not.toBeNull();
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
