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
 * Tests for the right-rail anchor minimap that replaced
 * `TimelineChapterStrip` in issue #2052. The minimap is a pure
 * component driven by props, so the tests mount it directly with
 * stubbed `onSelectAnchor` and assert on rendered DOM + callback
 * shape — same coverage shape as the strip's tests, plus the new
 * current-position highlight + empty-state contract.
 */

import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { SessionAnchorMinimap } from '../SessionAnchorMinimap';

import type { SessionAnchor } from '@/api/types';

afterEach(() => {
  cleanup();
});

function anchor(
  id: number,
  name: string,
  byteOffset: number,
  entryCount: number,
  timestamp = '2026-04-30T00:00:00Z',
): SessionAnchor {
  return {
    anchor_id: id,
    byte_offset: byteOffset,
    name,
    timestamp,
    entry_count_in_segment: entryCount,
  };
}

describe('SessionAnchorMinimap', () => {
  it('renders_one_row_per_anchor', () => {
    const anchors = [anchor(1, 'N1', 0, 3), anchor(2, 'N2', 100, 5), anchor(3, 'N3', 250, 7)];
    render(
      <SessionAnchorMinimap anchors={anchors} currentAnchorId={null} onSelectAnchor={() => {}} />,
    );

    const rows = screen.getAllByTestId('anchor-minimap-row');
    expect(rows).toHaveLength(3);

    expect(rows[0]).toHaveTextContent('N1');
    expect(rows[1]).toHaveTextContent('N2');
    expect(rows[2]).toHaveTextContent('N3');

    const counts = screen.getAllByTestId('anchor-minimap-row-count');
    expect(counts.map((b) => b.textContent)).toEqual(['3', '5', '7']);
  });

  it('truncates_long_names_but_preserves_full_name_on_title', () => {
    const longName = 'daily-summary-checkpoint-2026-04-28';
    render(
      <SessionAnchorMinimap
        anchors={[anchor(1, longName, 0, 1)]}
        currentAnchorId={null}
        onSelectAnchor={() => {}}
      />,
    );
    const row = screen.getByTestId('anchor-minimap-row');
    expect(row).toHaveAttribute('title', longName);
    expect(row.textContent).toContain('…');
    expect(row.textContent).not.toContain(longName);
  });

  it('click_invokes_callback_with_from_to_pair', () => {
    const onSelect = vi.fn();
    const a1 = anchor(1, 'A1', 0, 2);
    const a2 = anchor(2, 'A2', 100, 4);
    const a3 = anchor(3, 'A3', 220, 6);

    render(
      <SessionAnchorMinimap
        anchors={[a1, a2, a3]}
        currentAnchorId={null}
        onSelectAnchor={onSelect}
      />,
    );

    const rows = screen.getAllByTestId('anchor-minimap-row');
    fireEvent.click(rows[1]!); // A2

    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onSelect).toHaveBeenCalledWith(a2, a3);
  });

  it('most_recent_row_passes_null_to_anchor', () => {
    const onSelect = vi.fn();
    const a1 = anchor(1, 'A1', 0, 2);
    const a2 = anchor(2, 'A2', 100, 4);
    const a3 = anchor(3, 'A3', 220, 6);

    render(
      <SessionAnchorMinimap
        anchors={[a1, a2, a3]}
        currentAnchorId={null}
        onSelectAnchor={onSelect}
      />,
    );

    const rows = screen.getAllByTestId('anchor-minimap-row');
    fireEvent.click(rows[2]!);

    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onSelect).toHaveBeenCalledWith(a3, null);
  });

  it('current_position_highlights_explicit_anchor_id', () => {
    const a1 = anchor(1, 'A1', 0, 2);
    const a2 = anchor(2, 'A2', 100, 4);
    const a3 = anchor(3, 'A3', 220, 6);

    render(
      <SessionAnchorMinimap anchors={[a1, a2, a3]} currentAnchorId={2} onSelectAnchor={() => {}} />,
    );

    const rows = screen.getAllByTestId('anchor-minimap-row');
    expect(rows[0]).not.toHaveAttribute('data-current');
    expect(rows[1]).toHaveAttribute('data-current', 'true');
    expect(rows[1]).toHaveAttribute('aria-current', 'true');
    expect(rows[2]).not.toHaveAttribute('data-current');
  });

  it('current_position_defaults_to_most_recent_when_unset', () => {
    const a1 = anchor(1, 'A1', 0, 2);
    const a2 = anchor(2, 'A2', 100, 4);
    const a3 = anchor(3, 'A3', 220, 6);

    render(
      <SessionAnchorMinimap
        anchors={[a1, a2, a3]}
        currentAnchorId={null}
        onSelectAnchor={() => {}}
      />,
    );

    const rows = screen.getAllByTestId('anchor-minimap-row');
    // Latest entry in append order is "you are here" by default — the
    // user reads top-down and expects the bottom row to be the live tip.
    expect(rows[2]).toHaveAttribute('data-current', 'true');
  });

  it('renders_empty_state_when_no_anchors', () => {
    render(<SessionAnchorMinimap anchors={[]} currentAnchorId={null} onSelectAnchor={() => {}} />);

    expect(screen.getByTestId('anchor-minimap-empty')).toBeInTheDocument();
    expect(screen.queryByTestId('anchor-minimap')).not.toBeInTheDocument();
    expect(screen.queryAllByTestId('anchor-minimap-row')).toHaveLength(0);
  });

  it('groups_rows_by_day_bucket_header', () => {
    // Two anchors on the same day → one header; a third on a different
    // day → a second header. The bucket reducer walks once and emits a
    // header only when the bucket changes.
    const sameDayA = anchor(1, 'A1', 0, 2, '2026-04-30T01:00:00Z');
    const sameDayB = anchor(2, 'A2', 100, 4, '2026-04-30T05:00:00Z');
    const otherDay = anchor(3, 'A3', 220, 6, '2026-04-29T01:00:00Z');

    render(
      <SessionAnchorMinimap
        anchors={[otherDay, sameDayA, sameDayB]}
        currentAnchorId={null}
        onSelectAnchor={() => {}}
      />,
    );

    const buckets = screen.getAllByTestId('anchor-minimap-bucket');
    expect(buckets).toHaveLength(2);
  });
});
