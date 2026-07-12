/*
 * Copyright 2026 Rararulab
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

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { expectedCandleStreamHealthForGroups } from '../finance-candle-health';

import type { CandleStream } from '@/api/data-feeds';

const NOW = new Date('2026-07-12T12:00:00Z');

function candleStream(partial: Partial<CandleStream> = {}): CandleStream {
  return {
    source_name: partial.source_name ?? 'finance-binance-market-candles',
    venue: partial.venue ?? 'binance',
    symbol: partial.symbol ?? 'BTCUSDT',
    timeframe: partial.timeframe ?? '1m',
    candle_count: partial.candle_count ?? 120,
    first_open_time: partial.first_open_time ?? '2026-07-12T10:00:00Z',
    latest_open_time: partial.latest_open_time ?? '2026-07-12T11:59:00Z',
    latest_close_time: partial.latest_close_time ?? '2026-07-12T11:59:59Z',
    latest_ingested_at: partial.latest_ingested_at ?? '2026-07-12T12:00:01Z',
  };
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(NOW);
});

afterEach(() => {
  vi.useRealTimers();
});

describe('expectedCandleStreamHealthForGroups', () => {
  it('deduplicates overlapping expected selectors before reporting freshness', () => {
    const health = expectedCandleStreamHealthForGroups(
      [
        {
          sourceName: 'finance-binance-market-candles',
          venue: 'binance',
          symbols: ['BTCUSDT', 'btcusdt'],
          timeframes: ['1m', '1M'],
        },
      ],
      [candleStream()],
    );

    expect(health).toEqual({
      status: 'fresh',
      label: 'Fresh',
      detail: '1/1 expected stream fresh.',
    });
  });

  it('keeps multiple subscription groups separate instead of creating a global cross product', () => {
    const health = expectedCandleStreamHealthForGroups(
      [
        {
          sourceName: 'finance-binance-market-candles',
          venue: 'binance',
          symbols: ['BTCUSDT'],
          timeframes: ['1m'],
        },
        {
          sourceName: 'finance-binance-market-candles',
          venue: 'binance',
          symbols: ['ETHUSDT'],
          timeframes: ['5m'],
        },
      ],
      [candleStream(), candleStream({ symbol: 'ETHUSDT', timeframe: '5m' })],
    );

    expect(health).toEqual({
      status: 'fresh',
      label: 'Fresh',
      detail: '2/2 expected streams fresh.',
    });
  });

  it('reports partial missing coverage when only some expected streams exist', () => {
    const health = expectedCandleStreamHealthForGroups(
      [
        {
          sourceName: 'finance-binance-market-candles',
          venue: 'binance',
          symbols: ['BTCUSDT', 'ETHUSDT'],
          timeframes: ['1m'],
        },
      ],
      [candleStream({ symbol: 'BTCUSDT' })],
    );

    expect(health).toEqual({
      status: 'missing',
      label: 'Partial',
      detail: '1/2 expected streams present; 1 missing.',
    });
  });

  it('reports partial stale coverage when all expected streams exist but some are stale', () => {
    const health = expectedCandleStreamHealthForGroups(
      [
        {
          sourceName: 'finance-binance-market-candles',
          venue: 'binance',
          symbols: ['BTCUSDT', 'ETHUSDT'],
          timeframes: ['1m'],
        },
      ],
      [
        candleStream({ symbol: 'BTCUSDT' }),
        candleStream({
          symbol: 'ETHUSDT',
          latest_open_time: '2026-07-12T11:50:00Z',
          latest_close_time: '2026-07-12T11:50:59Z',
        }),
      ],
    );

    expect(health).toEqual({
      status: 'stale',
      label: 'Partial',
      detail: '1/2 expected streams fresh; 1 stale.',
    });
  });

  it('falls back to broad selector matching when a group cannot enumerate expected streams', () => {
    const health = expectedCandleStreamHealthForGroups(
      [
        {
          sourceName: 'finance-binance-market-candles',
          venue: 'binance',
          symbols: [],
          timeframes: [],
        },
      ],
      [candleStream({ symbol: 'ETHUSDT', timeframe: '5m' })],
    );

    expect(health).toEqual({
      status: 'fresh',
      label: 'Fresh',
      detail: '1/1 matched stream fresh.',
    });
  });
});
