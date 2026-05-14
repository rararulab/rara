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
 * BDD bindings for `specs/issue-2087-response-footer-trace-cascade.spec.md`.
 *
 * Each `it(...)` carries the spec's `Filter:` selector verbatim so
 * `agent-spec verify` can resolve scenarios to real test functions once
 * the vitest adapter (issue 2015) lands. Bindings for spec 2023 (the
 * superseded three-dot menu + tool-row cascade design) have been removed
 * — see PR for issue 2087.
 */

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { RaraTurnCard } from '../RaraTurnCard';
import type { TurnCardData } from '../TurnCard';

import { ApiError } from '@/api/client';

import '@/i18n';

// Mock the fetch wrappers so the tests exercise the wiring, not the network.
const fetchExecutionTraceMock = vi.fn();
const fetchCascadeTraceMock = vi.fn();
vi.mock('@/api/sessions', () => ({
  fetchExecutionTrace: (...args: unknown[]) => fetchExecutionTraceMock(...args),
  fetchCascadeTrace: (...args: unknown[]) => fetchCascadeTraceMock(...args),
}));

function makeTurn(overrides: Partial<TurnCardData> = {}): TurnCardData {
  return {
    id: 'turn-42',
    text: 'final assistant text',
    reasoning: '',
    toolCalls: [
      {
        id: 'tool-1',
        name: 'shell',
        result: { success: true, preview: 'ok', error: null },
      },
    ],
    markers: [],
    metrics: null,
    usage: null,
    inFlight: false,
    createdAt: 1_700_000_000_000,
    finalSeq: 42,
    ...overrides,
  };
}

function renderCard(turn: TurnCardData, sessionKey = 'sess-abc') {
  // Each test gets a fresh QueryClient so cached results from earlier
  // scenarios cannot leak into the next one.
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0, staleTime: 0 } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <RaraTurnCard turn={turn} sessionKey={sessionKey} />
    </QueryClientProvider>,
  );
}

afterEach(() => {
  cleanup();
  fetchExecutionTraceMock.mockReset();
  fetchCascadeTraceMock.mockReset();
});

describe('RaraTurnCard — trace + cascade affordances', () => {
  beforeEach(() => {
    fetchExecutionTraceMock.mockReset();
    fetchCascadeTraceMock.mockReset();
  });

  // BDD: pure_text_turn_shows_trace_and_cascade_buttons
  // Falsifier: revert the `renderResponseActions` slot in RaraTurnCard;
  // pure-text turns render no footer buttons and both assertions fail.
  it('RaraTurnCard__pure_text_turn_shows_trace_and_cascade_buttons', () => {
    renderCard(
      makeTurn({
        finalSeq: 42,
        inFlight: false,
        text: '2 + 2 = 4',
        reasoning: '',
        toolCalls: [],
      }),
    );
    expect(screen.getByRole('button', { name: 'Trace' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Cascade' })).toBeInTheDocument();
  });

  // BDD: tool_call_turn_shows_trace_and_cascade_buttons
  // Falsifier: gate the slot on `toolCalls.length === 0`; turns with tool
  // calls render no buttons and both assertions fail.
  it('RaraTurnCard__tool_call_turn_shows_trace_and_cascade_buttons', () => {
    renderCard(makeTurn({ finalSeq: 42, inFlight: false, text: 'done' }));
    expect(screen.getByRole('button', { name: 'Trace' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Cascade' })).toBeInTheDocument();
  });

  // BDD: trace_and_cascade_buttons_suppressed_when_inflight_or_seq_null
  // Falsifier: drop the `inspectable` gate; live / seq-less turns render
  // the buttons and both queryByText calls return non-null.
  it('RaraTurnCard__trace_and_cascade_buttons_suppressed_when_inflight_or_seq_null', () => {
    renderCard(makeTurn({ finalSeq: null, inFlight: true }));
    expect(screen.queryByText(/^Trace$/)).toBeNull();
    expect(screen.queryByText(/^Cascade$/)).toBeNull();
  });

  // BDD: trace_button_opens_trace_modal
  // Falsifier: stop wiring the Trace button to setTraceOpen; the modal
  // never appears and the findByRole('dialog') times out.
  it('RaraTurnCard__trace_button_opens_trace_modal', async () => {
    fetchExecutionTraceMock.mockResolvedValue({
      duration_secs: 1.23,
      iterations: 7,
      model: 'gpt-fixture-v9',
      input_tokens: 100,
      output_tokens: 200,
      thinking_ms: 50,
      thinking_preview: '',
      plan_steps: [],
      tools: [],
      rara_turn_id: 'turn-42',
    });

    renderCard(makeTurn({ finalSeq: 42, inFlight: false }));
    fireEvent.click(screen.getByRole('button', { name: 'Trace' }));

    const dialog = await screen.findByRole('dialog');
    await waitFor(() => {
      expect(within(dialog).getByText('gpt-fixture-v9')).toBeInTheDocument();
    });
    expect(within(dialog).getByText('7')).toBeInTheDocument();
    expect(fetchExecutionTraceMock).toHaveBeenCalledWith(
      'sess-abc',
      42,
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
  });

  // BDD: cascade_button_opens_cascade_modal
  // Falsifier: stop wiring the Cascade button to setCascadeOpen; the
  // modal never appears and the findByRole('dialog') times out.
  it('RaraTurnCard__cascade_button_opens_cascade_modal', async () => {
    fetchCascadeTraceMock.mockResolvedValue({
      message_id: 'sess-abc-42',
      ticks: [
        {
          index: 0,
          entries: [
            {
              id: 'cascade.0-aaa-1',
              kind: 'thought',
              content: 'cascade-fixture-thought',
              timestamp: '2026-04-30T00:00:00Z',
            },
          ],
        },
      ],
      summary: { tick_count: 1, tool_call_count: 1, total_entries: 1 },
    });

    renderCard(makeTurn({ finalSeq: 42, inFlight: false }));
    fireEvent.click(screen.getByRole('button', { name: 'Cascade' }));

    const dialog = await screen.findByRole('dialog');
    await waitFor(() => {
      expect(within(dialog).getByText(/cascade-fixture-thought/)).toBeInTheDocument();
    });
    expect(fetchCascadeTraceMock).toHaveBeenCalledWith(
      'sess-abc',
      42,
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
  });

  // BDD: tool_row_click_does_not_open_cascade
  // Falsifier: re-add `onOpenActivityDetails` wiring to RaraTurnCard;
  // clicking the tool row opens the cascade modal and the assertion
  // queryByRole('dialog') becomes non-null.
  it('RaraTurnCard__tool_row_click_does_not_open_cascade', async () => {
    renderCard(makeTurn({ finalSeq: 42, inFlight: false }));
    // Expand the activity section first so the tool row enters the DOM.
    const chevron = document.querySelector('.lucide-chevron-right');
    expect(chevron).not.toBeNull();
    const toggleBtn = (chevron as Element).closest('button');
    expect(toggleBtn).not.toBeNull();
    fireEvent.click(toggleBtn as HTMLElement);

    const toolNameNode = await screen.findByText('shell');
    fireEvent.click(toolNameNode);
    // No modal should mount from the tool-row click.
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(fetchCascadeTraceMock).not.toHaveBeenCalled();
  });

  it('RaraTurnCard__assistant_response_uses_timeline_scroll_not_inner_scrollbar', () => {
    const { container } = renderCard(
      makeTurn({
        text: Array.from({ length: 80 }, (_, idx) => `response line ${String(idx + 1)}`).join('\n'),
      }),
    );

    const response = container.querySelector('[data-search-root="response"]');
    expect(response).not.toBeNull();
    expect(response).not.toHaveClass('overflow-y-auto');
    expect(response).not.toHaveClass('scrollbar-hover');
    expect((response as HTMLElement).style.maxHeight).toBe('');
  });

  it('RaraTurnCard__assistant_response_renders_as_page_flow_not_message_bubble', () => {
    const { container } = renderCard(makeTurn({ text: 'final assistant text' }));

    const response = container.querySelector('[data-search-root="response"]');
    expect(response).not.toBeNull();
    expect(response).toHaveClass('px-0');
    expect(response).toHaveClass('py-1');
    expect(response?.closest('.shadow-minimal')).toBeNull();
    expect(response?.closest('.rounded-\\[8px\\]')).toBeNull();
  });

  // BDD bindings for `specs/issue-2031-thinking-only-turn-render.spec.md`.
  // The vendor `TurnCard` suppresses turns whose only activity is
  // `type: 'thinking'` (`hasNoMeaningfulWork` gate); the adapter reroutes
  // thinking-only turns to `type: 'intermediate'` so the card renders.

  it('thinking_only_turn_renders_card', async () => {
    renderCard(
      makeTurn({
        text: '',
        reasoning: 'let me think about this',
        toolCalls: [],
        inFlight: false,
      }),
    );
    const wrapper = document.querySelector('[data-turn-id="turn-42"]');
    expect(wrapper).not.toBeNull();
    // Activities are collapsed by default; expand to expose the reasoning
    // row so the test can assert the reasoning content reached the DOM.
    const chevron = document.querySelector('.lucide-chevron-right');
    expect(chevron).not.toBeNull();
    fireEvent.click((chevron as Element).closest('button') as HTMLElement);
    await waitFor(() => {
      expect(screen.getByText(/let me think about this/)).toBeInTheDocument();
    });
  });

  it('thinking_only_live_turn_renders_card', () => {
    renderCard(
      makeTurn({
        text: '',
        reasoning: 'working it out',
        toolCalls: [],
        inFlight: true,
        finalSeq: null,
      }),
    );
    const wrapper = document.querySelector('[data-turn-id="turn-42"]');
    expect(wrapper).not.toBeNull();
  });

  it('turn_with_reasoning_and_text_renders_both', async () => {
    renderCard(
      makeTurn({
        text: 'here is the answer',
        reasoning: 'step 1: ...',
        toolCalls: [],
        inFlight: false,
      }),
    );
    const wrapper = document.querySelector('[data-turn-id="turn-42"]');
    expect(wrapper).not.toBeNull();
    expect(screen.getByText('here is the answer')).toBeInTheDocument();
    const chevron = document.querySelector('.lucide-chevron-right');
    expect(chevron).not.toBeNull();
    fireEvent.click((chevron as Element).closest('button') as HTMLElement);
    await waitFor(() => {
      const expanded = document.querySelector('[data-search-exclude="true"]');
      expect(expanded?.textContent).toMatch(/Thinking|Processing/);
    });
  });

  it('empty_turn_remains_suppressed', () => {
    renderCard(
      makeTurn({
        text: '',
        reasoning: '',
        toolCalls: [],
        inFlight: false,
      }),
    );
    expect(document.querySelector('[data-turn-id="turn-42"]')).toBeNull();
  });

  // Friendly 404 / non-404 / generic error coverage — same modal,
  // now reached via the Trace button instead of the three-dot menu.
  it('RaraTurnCard__trace_modal_friendly_404', async () => {
    fetchExecutionTraceMock.mockRejectedValue(
      new ApiError(404, 'user message at seq 22 has no rara_turn_id metadata'),
    );

    renderCard(makeTurn({ finalSeq: 22, inFlight: false }));
    fireEvent.click(screen.getByRole('button', { name: 'Trace' }));

    const dialog = await screen.findByRole('dialog');
    await waitFor(() => {
      expect(within(dialog).getByRole('alert')).toBeInTheDocument();
    });
    expect(
      within(dialog).getByText(/Trace data is not available for this turn yet/i),
    ).toBeInTheDocument();
    expect(within(dialog).queryByText(/rara_turn_id metadata/i)).toBeNull();
  });

  it('RaraTurnCard__trace_modal_non_404_error_distinct', async () => {
    fetchExecutionTraceMock.mockRejectedValue(new ApiError(500, 'internal error'));

    renderCard(makeTurn({ finalSeq: 22, inFlight: false }));
    fireEvent.click(screen.getByRole('button', { name: 'Trace' }));

    const dialog = await screen.findByRole('dialog');
    await waitFor(() => {
      expect(within(dialog).getByRole('alert')).toBeInTheDocument();
    });
    expect(within(dialog).getByText(/internal error/i)).toBeInTheDocument();
    expect(within(dialog).queryByText(/Trace data is not available for this turn yet/i)).toBeNull();
  });

  it('RaraTurnCard__trace_modal_shows_error_on_fetch_failure', async () => {
    fetchExecutionTraceMock.mockRejectedValue(new Error('network is down'));

    renderCard(makeTurn({ finalSeq: 42, inFlight: false }));
    fireEvent.click(screen.getByRole('button', { name: 'Trace' }));

    const dialog = await screen.findByRole('dialog');
    await waitFor(() => {
      expect(within(dialog).getByRole('alert')).toBeInTheDocument();
    });
    expect(within(dialog).getByText(/network is down/i)).toBeInTheDocument();
    // The card itself is still rendered alongside the modal.
    expect(screen.getByText('final assistant text')).toBeInTheDocument();
  });
});
