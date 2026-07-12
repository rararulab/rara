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
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { FinanceWatchesCard } from '../FinanceWatchesCard';

import type {
  CandleStreamsResponse,
  FeedCatalogEntry,
  FinanceSubscription,
  FinanceSubscriptionsResponse,
} from '@/api/data-feeds';

const catalogMock = vi.fn();
const financeSubscriptionsMock = vi.fn();
const candleStreamsMock = vi.fn();
const enableCatalogEntryMock = vi.fn();
const createFinanceSubscriptionMock = vi.fn();
const unsubscribeCatalogEntryMock = vi.fn();
const openSettingsMock = vi.fn();

vi.mock('@/api/data-feeds', () => ({
  CANDLE_STREAM_OVERVIEW_LIMIT: 500,
  dataFeedsApi: {
    catalog: (...args: unknown[]) => catalogMock(...args),
    financeSubscriptions: (...args: unknown[]) => financeSubscriptionsMock(...args),
    candleStreams: (...args: unknown[]) => candleStreamsMock(...args),
    enableCatalogEntry: (...args: unknown[]) => enableCatalogEntryMock(...args),
    createFinanceSubscription: (...args: unknown[]) => createFinanceSubscriptionMock(...args),
    unsubscribeCatalogEntry: (...args: unknown[]) => unsubscribeCatalogEntryMock(...args),
  },
}));

vi.mock('@/components/settings/SettingsModalContext', () => ({
  useSettingsModal: () => ({
    openSettings: openSettingsMock,
    closeSettings: vi.fn(),
  }),
}));

function buildClient() {
  return new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0, staleTime: 0 },
      mutations: { retry: false },
    },
  });
}

function renderCard(sessionKey = 'session-1') {
  const client = buildClient();
  const utils = render(
    <QueryClientProvider client={client}>
      <FinanceWatchesCard sessionKey={sessionKey} />
    </QueryClientProvider>,
  );
  return { ...utils, client };
}

function catalogEntry(partial: Partial<FeedCatalogEntry> & { id: string }): FeedCatalogEntry {
  const entry: FeedCatalogEntry = {
    id: partial.id,
    name: partial.name ?? partial.id,
    description: partial.description ?? 'source description',
    feed_type: partial.feed_type ?? 'rss',
    tags: partial.tags ?? ['finance', 'news'],
    enabled: partial.enabled ?? true,
    feed_id: partial.feed_id ?? null,
    requires_configuration: partial.requires_configuration ?? false,
    setup_hint: partial.setup_hint ?? null,
    transport_template: partial.transport_template ?? null,
  };
  if (partial.source_name !== undefined) entry.source_name = partial.source_name;
  if (partial.provider !== undefined) entry.provider = partial.provider;
  if (partial.venue !== undefined) entry.venue = partial.venue;
  if (partial.configured_symbols !== undefined)
    entry.configured_symbols = partial.configured_symbols;
  if (partial.configured_timeframes !== undefined) {
    entry.configured_timeframes = partial.configured_timeframes;
  }
  if (partial.subscriptions !== undefined) entry.subscriptions = partial.subscriptions;
  return entry;
}

function financeSubscription(partial: Partial<FinanceSubscription> = {}): FinanceSubscription {
  return {
    subscription_id: partial.subscription_id ?? 'sub-1',
    session_key: partial.session_key ?? 'session-1',
    event_kinds: partial.event_kinds ?? ['market_candle_closed'],
    source_names: partial.source_names ?? ['finance-binance-market-candles'],
    matches_all_sources: partial.matches_all_sources ?? false,
    sources: partial.sources ?? [],
    category_tags: partial.category_tags ?? [],
    watch_terms: partial.watch_terms ?? [],
    venues: partial.venues ?? ['binance'],
    symbols: partial.symbols ?? ['BTCUSDT', 'ETHUSDT'],
    timeframes: partial.timeframes ?? ['1m'],
    delivery: partial.delivery ?? 'silent',
    cooldown_secs: partial.cooldown_secs ?? 900,
    max_immediate_per_hour: partial.max_immediate_per_hour ?? 6,
  };
}

beforeEach(() => {
  catalogMock.mockReset();
  financeSubscriptionsMock.mockReset();
  candleStreamsMock.mockReset();
  enableCatalogEntryMock.mockReset();
  createFinanceSubscriptionMock.mockReset();
  unsubscribeCatalogEntryMock.mockReset();
  openSettingsMock.mockReset();
  candleStreamsMock.mockResolvedValue({
    streams: [],
    count: 0,
    query_limit: 100,
    has_more: false,
  } satisfies CandleStreamsResponse);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('FinanceWatchesCard', () => {
  it('shows_provider_metadata_for_catalog_watch_sources', async () => {
    catalogMock.mockResolvedValue([
      catalogEntry({
        id: 'binance-market-candles',
        name: 'Binance Market Candles',
        feed_type: 'market_candle',
        provider: 'binance',
        venue: 'binance',
        configured_symbols: ['BTCUSDT', 'ETHUSDT'],
        configured_timeframes: ['1m'],
      }),
      catalogEntry({
        id: 'longbridge-market-candles',
        name: 'Longbridge Market Data',
        feed_type: 'market_candle',
        provider: 'longbridge',
        enabled: false,
        requires_configuration: true,
        venue: 'longbridge',
        configured_symbols: ['AAPL.US', 'NVDA.US'],
        configured_timeframes: ['1d'],
      }),
    ]);
    financeSubscriptionsMock.mockResolvedValue({
      subscriptions: [],
      count: 0,
    } satisfies FinanceSubscriptionsResponse);

    renderCard('session-1');

    expect(await screen.findByText('Provider binance')).toBeInTheDocument();
    expect(screen.getByText('Provider longbridge')).toBeInTheDocument();
  });

  it('creates_session_watch_from_catalog_kline_source_with_configured_selectors', async () => {
    const binance = catalogEntry({
      id: 'binance-market-candles',
      name: 'Binance Market Candles',
      feed_type: 'market_candle',
      source_name: 'finance-binance-market-candles',
      venue: 'binance',
      configured_symbols: ['BTCUSDT', 'ETHUSDT'],
      configured_timeframes: ['1m'],
      tags: ['finance', 'market-data', 'crypto', 'binance'],
      transport_template: {
        venue: 'binance',
        symbols: ['BTCUSDT', 'ETHUSDT'],
        timeframes: ['1m'],
      },
    });
    catalogMock.mockResolvedValue([binance]);
    financeSubscriptionsMock.mockResolvedValue({
      subscriptions: [],
      count: 0,
    } satisfies FinanceSubscriptionsResponse);
    createFinanceSubscriptionMock.mockResolvedValue({
      created: true,
      subscription: {
        subscription_id: 'sub-1',
        session_key: 'session-1',
        event_kinds: ['market_candle_closed'],
        source_names: ['finance-binance-market-candles'],
        matches_all_sources: false,
        sources: [],
        category_tags: [],
        watch_terms: [],
        venues: ['binance'],
        symbols: ['BTCUSDT', 'ETHUSDT'],
        timeframes: ['1m'],
        delivery: 'silent',
        cooldown_secs: 900,
        max_immediate_per_hour: 6,
      },
    });

    renderCard('session-1');

    const button = await screen.findByRole('button', { name: 'Watch' });
    fireEvent.click(button);

    await waitFor(() => {
      expect(createFinanceSubscriptionMock).toHaveBeenCalledWith({
        session_key: 'session-1',
        catalog_source_ids: ['binance-market-candles'],
        delivery: 'silent',
        venues: ['binance'],
        symbols: ['BTCUSDT', 'ETHUSDT'],
        timeframes: ['1m'],
      });
    });
  });

  it('enables_ready_source_before_allowing_session_watch', async () => {
    catalogMock.mockResolvedValue([
      catalogEntry({
        id: 'fed-press-releases',
        name: 'Federal Reserve Press Releases',
        source_name: 'finance-fed-press-releases',
        enabled: false,
      }),
    ]);
    financeSubscriptionsMock.mockResolvedValue({
      subscriptions: [],
      count: 0,
    } satisfies FinanceSubscriptionsResponse);
    enableCatalogEntryMock.mockResolvedValue({
      id: 'feed-1',
      name: 'finance-fed-press-releases',
      feed_type: 'rss',
      tags: ['finance', 'news'],
      transport: {},
      auth: null,
      enabled: true,
      status: 'idle',
      last_error: null,
      created_at: '2026-01-01T00:00:00Z',
      updated_at: '2026-01-01T00:00:00Z',
    });

    renderCard('session-1');

    fireEvent.click(await screen.findByRole('button', { name: 'Enable source' }));

    await waitFor(() => {
      expect(enableCatalogEntryMock).toHaveBeenCalledWith('fed-press-releases');
    });
    expect(createFinanceSubscriptionMock).not.toHaveBeenCalled();
  });

  it('removes_existing_session_watch_by_subscription_id', async () => {
    catalogMock.mockResolvedValue([
      catalogEntry({
        id: 'fed-press-releases',
        name: 'Federal Reserve Press Releases',
        source_name: 'finance-fed-press-releases',
      }),
    ]);
    financeSubscriptionsMock.mockResolvedValue({
      count: 1,
      subscriptions: [
        {
          subscription_id: 'sub-1',
          session_key: 'session-1',
          event_kinds: ['rss_article'],
          source_names: ['finance-fed-press-releases'],
          matches_all_sources: false,
          sources: [],
          category_tags: [],
          watch_terms: [],
          venues: [],
          symbols: [],
          timeframes: [],
          delivery: 'silent',
          cooldown_secs: 900,
          max_immediate_per_hour: 6,
        },
      ],
    } satisfies FinanceSubscriptionsResponse);
    unsubscribeCatalogEntryMock.mockResolvedValue({
      catalog_source_id: 'fed-press-releases',
      source_name: 'finance-fed-press-releases',
      removed_subscription_ids: ['sub-1'],
      removed_count: 1,
      remaining_subscription_ids: [],
    });

    renderCard('session-1');

    expect(await screen.findByText('watching')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Unwatch' }));

    await waitFor(() => {
      expect(unsubscribeCatalogEntryMock).toHaveBeenCalledWith('fed-press-releases', {
        subscription_ids: ['sub-1'],
      });
    });
  });

  it('opens_data_feed_settings_for_sources_that_require_configuration', async () => {
    catalogMock.mockResolvedValue([
      catalogEntry({
        id: 'longport-market-candles',
        name: 'LongPort Market Candles',
        feed_type: 'market_candle',
        enabled: false,
        requires_configuration: true,
        setup_hint: 'Configure LongPort credentials and selectors before enabling.',
      }),
    ]);
    financeSubscriptionsMock.mockResolvedValue({
      subscriptions: [],
      count: 0,
    } satisfies FinanceSubscriptionsResponse);

    renderCard('session-1');

    fireEvent.click(await screen.findByRole('button', { name: 'Configure' }));

    expect(openSettingsMock).toHaveBeenCalledWith('data-feeds', {
      dataFeedCatalogId: 'longport-market-candles',
    });
    expect(createFinanceSubscriptionMock).not.toHaveBeenCalled();
    expect(enableCatalogEntryMock).not.toHaveBeenCalled();
  });

  it('shows_fresh_stream_health_for_watched_kline_source', async () => {
    vi.spyOn(Date, 'now').mockReturnValue(Date.parse('2026-07-12T00:42:30Z'));
    catalogMock.mockResolvedValue([
      catalogEntry({
        id: 'binance-market-candles',
        name: 'Binance Market Candles',
        feed_type: 'market_candle',
        source_name: 'finance-binance-market-candles',
        venue: 'binance',
        configured_symbols: ['BTCUSDT', 'ETHUSDT'],
        configured_timeframes: ['1m'],
        tags: ['finance', 'market-data', 'crypto', 'binance'],
        transport_template: {
          venue: 'binance',
          symbols: ['BTCUSDT', 'ETHUSDT'],
          timeframes: ['1m'],
        },
      }),
    ]);
    financeSubscriptionsMock.mockResolvedValue({
      subscriptions: [financeSubscription()],
      count: 1,
    } satisfies FinanceSubscriptionsResponse);
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
        {
          source_name: 'finance-binance-market-candles',
          venue: 'binance',
          symbol: 'ETHUSDT',
          timeframe: '1m',
          candle_count: 42,
          first_open_time: '2026-07-12T00:00:00Z',
          latest_open_time: '2026-07-12T00:41:00Z',
          latest_close_time: '2026-07-12T00:41:59Z',
          latest_ingested_at: '2026-07-12T00:42:02Z',
        },
      ],
      count: 2,
      query_limit: 100,
      has_more: false,
    } satisfies CandleStreamsResponse);

    renderCard('session-1');

    expect(await screen.findByText('watching')).toBeInTheDocument();
    expect(await screen.findByText('Data Fresh')).toBeInTheDocument();
    expect(screen.getByText('2/2 expected streams fresh.')).toBeInTheDocument();
    await waitFor(() => {
      expect(candleStreamsMock).toHaveBeenCalledWith({ limit: 500 });
    });
  });

  it('shows_partial_stream_health_when_a_watched_kline_symbol_has_no_stored_stream', async () => {
    vi.spyOn(Date, 'now').mockReturnValue(Date.parse('2026-07-12T00:42:30Z'));
    catalogMock.mockResolvedValue([
      catalogEntry({
        id: 'binance-market-candles',
        name: 'Binance Market Candles',
        feed_type: 'market_candle',
        source_name: 'finance-binance-market-candles',
        venue: 'binance',
        configured_symbols: ['BTCUSDT', 'ETHUSDT'],
        configured_timeframes: ['1m'],
        tags: ['finance', 'market-data', 'crypto', 'binance'],
        transport_template: {
          venue: 'binance',
          symbols: ['BTCUSDT', 'ETHUSDT'],
          timeframes: ['1m'],
        },
      }),
    ]);
    financeSubscriptionsMock.mockResolvedValue({
      subscriptions: [financeSubscription()],
      count: 1,
    } satisfies FinanceSubscriptionsResponse);
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
      has_more: false,
    } satisfies CandleStreamsResponse);

    renderCard('session-1');

    expect(await screen.findByText('watching')).toBeInTheDocument();
    expect(await screen.findByText('Data Partial')).toBeInTheDocument();
    expect(
      screen.getByText('1/2 expected streams present; 1 missing. Missing: binance ETHUSDT 1m.'),
    ).toBeInTheDocument();
  });

  it('warns_when_watched_kline_health_uses_a_limited_stream_overview', async () => {
    vi.spyOn(Date, 'now').mockReturnValue(Date.parse('2026-07-12T00:42:30Z'));
    catalogMock.mockResolvedValue([
      catalogEntry({
        id: 'binance-market-candles',
        name: 'Binance Market Candles',
        feed_type: 'market_candle',
        source_name: 'finance-binance-market-candles',
        venue: 'binance',
        configured_symbols: ['BTCUSDT'],
        configured_timeframes: ['1m'],
        tags: ['finance', 'market-data', 'crypto', 'binance'],
        transport_template: {
          venue: 'binance',
          symbols: ['BTCUSDT'],
          timeframes: ['1m'],
        },
      }),
    ]);
    financeSubscriptionsMock.mockResolvedValue({
      subscriptions: [financeSubscription({ symbols: ['BTCUSDT'] })],
      count: 1,
    } satisfies FinanceSubscriptionsResponse);
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
      query_limit: 1,
      has_more: true,
    } satisfies CandleStreamsResponse);

    renderCard('session-1');

    expect(await screen.findByText('watching')).toBeInTheDocument();
    expect(
      screen.getByText(
        'K-line stream overview is limited to 1 streams; a missing health check may need a narrower source, symbol, or timeframe query.',
      ),
    ).toBeInTheDocument();
  });

  it('uses_subscription_selectors_for_watched_kline_health', async () => {
    vi.spyOn(Date, 'now').mockReturnValue(Date.parse('2026-07-12T00:42:30Z'));
    catalogMock.mockResolvedValue([
      catalogEntry({
        id: 'binance-market-candles',
        name: 'Binance Market Candles',
        feed_type: 'market_candle',
        source_name: 'finance-binance-market-candles',
        venue: 'binance',
        configured_symbols: ['BTCUSDT', 'ETHUSDT'],
        configured_timeframes: ['1m'],
        tags: ['finance', 'market-data', 'crypto', 'binance'],
        transport_template: {
          venue: 'binance',
          symbols: ['BTCUSDT', 'ETHUSDT'],
          timeframes: ['1m'],
        },
      }),
    ]);
    financeSubscriptionsMock.mockResolvedValue({
      subscriptions: [
        financeSubscription({
          symbols: ['BTCUSDT'],
          timeframes: ['1m'],
        }),
      ],
      count: 1,
    } satisfies FinanceSubscriptionsResponse);
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
      has_more: false,
    } satisfies CandleStreamsResponse);

    renderCard('session-1');

    expect(await screen.findByText('watching')).toBeInTheDocument();
    expect(await screen.findByText('Data Fresh')).toBeInTheDocument();
    expect(screen.getByText('1/1 expected stream fresh.')).toBeInTheDocument();
  });
});
