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

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import DataFeedsPanel from '../DataFeedsPanel';

import type {
  CandleStreamsResponse,
  DataFeedConfig,
  FeedCatalogEntry,
  FinanceSubscriptionsResponse,
} from '@/api/data-feeds';

const listMock = vi.fn();
const summariesMock = vi.fn();
const catalogMock = vi.fn();
const financeSubscriptionsMock = vi.fn();
const candleStreamsMock = vi.fn();

vi.mock('@/api/data-feeds', () => ({
  dataFeedsApi: {
    list: (...args: unknown[]) => listMock(...args),
    summaries: (...args: unknown[]) => summariesMock(...args),
    catalog: (...args: unknown[]) => catalogMock(...args),
    financeSubscriptions: (...args: unknown[]) => financeSubscriptionsMock(...args),
    candleStreams: (...args: unknown[]) => candleStreamsMock(...args),
    toggle: vi.fn(),
    delete: vi.fn(),
    enableCatalogEntry: vi.fn(),
    disableCatalogEntry: vi.fn(),
    unsubscribeCatalogEntry: vi.fn(),
    deleteFinanceSubscription: vi.fn(),
  },
}));

function buildClient() {
  return new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0, staleTime: 0 },
      mutations: { retry: false },
    },
  });
}

function renderPanel() {
  const client = buildClient();
  const utils = render(
    <QueryClientProvider client={client}>
      <DataFeedsPanel />
    </QueryClientProvider>,
  );
  return { ...utils, client };
}

function feed(partial: Partial<DataFeedConfig> = {}): DataFeedConfig {
  return {
    id: partial.id ?? 'feed-1',
    name: partial.name ?? 'finance-binance-market-candles',
    feed_type: partial.feed_type ?? 'market_candle',
    tags: partial.tags ?? ['finance', 'market-data'],
    transport: partial.transport ?? {},
    auth: partial.auth ?? null,
    enabled: partial.enabled ?? true,
    status: partial.status ?? 'running',
    last_error: partial.last_error ?? null,
    created_at: partial.created_at ?? '2026-07-01T00:00:00Z',
    updated_at: partial.updated_at ?? '2026-07-01T00:00:00Z',
  };
}

beforeEach(() => {
  listMock.mockReset();
  summariesMock.mockReset();
  catalogMock.mockReset();
  financeSubscriptionsMock.mockReset();
  candleStreamsMock.mockReset();

  listMock.mockResolvedValue([]);
  summariesMock.mockResolvedValue([]);
  catalogMock.mockResolvedValue([] satisfies FeedCatalogEntry[]);
  financeSubscriptionsMock.mockResolvedValue({
    subscriptions: [],
    count: 0,
  } satisfies FinanceSubscriptionsResponse);
  candleStreamsMock.mockResolvedValue({
    streams: [],
    count: 0,
    query_limit: 100,
  } satisfies CandleStreamsResponse);
});

afterEach(() => {
  cleanup();
});

describe('DataFeedsPanel', () => {
  it('requests and displays stored K-line stream watermarks', async () => {
    listMock.mockResolvedValue([feed()]);
    candleStreamsMock.mockResolvedValue({
      streams: [
        {
          source_name: 'finance-binance-market-candles',
          venue: 'binance',
          symbol: 'BTCUSDT',
          timeframe: '1m',
          candle_count: 42,
          first_open_time: '2026-07-12T00:00:00Z',
          latest_open_time: '2026-07-12T00:41:00Z',
          latest_close_time: '2026-07-12T00:41:59Z',
          latest_ingested_at: '2026-07-12T00:42:02Z',
        },
      ],
      count: 1,
      query_limit: 100,
    } satisfies CandleStreamsResponse);

    renderPanel();

    expect(await screen.findByText('Stored K-line streams')).toBeInTheDocument();
    expect(await screen.findByText('BINANCE · BTCUSDT · 1m')).toBeInTheDocument();
    expect(screen.getAllByText('finance-binance-market-candles').length).toBeGreaterThan(0);
    expect(screen.getByText('42')).toBeInTheDocument();

    await waitFor(() => {
      expect(candleStreamsMock).toHaveBeenCalledWith({ limit: 100 });
    });
  });

  it('shows an empty state before the first persisted candle', async () => {
    renderPanel();

    expect(await screen.findByText('Stored K-line streams')).toBeInTheDocument();
    expect(
      screen.getByText(
        'No closed candles have been stored yet. Enable a K-line feed and wait for the first persisted candle.',
      ),
    ).toBeInTheDocument();
  });
});
