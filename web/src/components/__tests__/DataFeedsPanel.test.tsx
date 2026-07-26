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

import DataFeedsPanel from '../DataFeedsPanel';

import type {
  CandleStreamsResponse,
  DataFeedConfig,
  FeedCatalogEntry,
  FeedEventsResponse,
  FinanceFeedBundlesResponse,
  FinanceSubscription,
  FinanceSubscriptionsResponse,
  MarketCandleFreshnessResponse,
  MarketCandleGapsResponse,
  MarketCandlesResponse,
} from '@/api/data-feeds';

const listMock = vi.fn();
const summariesMock = vi.fn();
const catalogMock = vi.fn();
const eventsMock = vi.fn();
const financeBundlesMock = vi.fn();
const financeSubscriptionsMock = vi.fn();
const candleStreamsMock = vi.fn();
const candlesMock = vi.fn();
const recentCandlesMock = vi.fn();
const candleFreshnessMock = vi.fn();
const candleGapsMock = vi.fn();
const enableCatalogEntryMock = vi.fn();

vi.mock('@/api/data-feeds', () => ({
  CANDLE_STREAM_OVERVIEW_LIMIT: 500,
  dataFeedsApi: {
    list: (...args: unknown[]) => listMock(...args),
    summaries: (...args: unknown[]) => summariesMock(...args),
    catalog: (...args: unknown[]) => catalogMock(...args),
    events: (...args: unknown[]) => eventsMock(...args),
    financeBundles: (...args: unknown[]) => financeBundlesMock(...args),
    financeSubscriptions: (...args: unknown[]) => financeSubscriptionsMock(...args),
    candleStreams: (...args: unknown[]) => candleStreamsMock(...args),
    candles: (...args: unknown[]) => candlesMock(...args),
    recentCandles: (...args: unknown[]) => recentCandlesMock(...args),
    candleFreshness: (...args: unknown[]) => candleFreshnessMock(...args),
    candleGaps: (...args: unknown[]) => candleGapsMock(...args),
    toggle: vi.fn(),
    delete: vi.fn(),
    enableCatalogEntry: (...args: unknown[]) => enableCatalogEntryMock(...args),
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

function candleSubscription(partial: Partial<FinanceSubscription> = {}): FinanceSubscription {
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
    symbols: partial.symbols ?? ['BTCUSDT'],
    timeframes: partial.timeframes ?? ['1m'],
    delivery: partial.delivery ?? 'silent',
    cooldown_secs: partial.cooldown_secs ?? 900,
    max_immediate_per_hour: partial.max_immediate_per_hour ?? 6,
  };
}

beforeEach(() => {
  if (!Element.prototype.hasPointerCapture) {
    Element.prototype.hasPointerCapture = () => false;
  }
  if (!Element.prototype.setPointerCapture) {
    Element.prototype.setPointerCapture = () => {};
  }
  if (!Element.prototype.releasePointerCapture) {
    Element.prototype.releasePointerCapture = () => {};
  }

  listMock.mockReset();
  summariesMock.mockReset();
  catalogMock.mockReset();
  eventsMock.mockReset();
  financeBundlesMock.mockReset();
  financeSubscriptionsMock.mockReset();
  candleStreamsMock.mockReset();
  candlesMock.mockReset();
  recentCandlesMock.mockReset();
  candleFreshnessMock.mockReset();
  candleGapsMock.mockReset();
  enableCatalogEntryMock.mockReset();

  listMock.mockResolvedValue([]);
  summariesMock.mockResolvedValue([]);
  catalogMock.mockResolvedValue([] satisfies FeedCatalogEntry[]);
  financeBundlesMock.mockResolvedValue({
    bundles: [],
    count: 0,
  } satisfies FinanceFeedBundlesResponse);
  eventsMock.mockResolvedValue({
    events: [],
    total: 0,
    has_more: false,
  } satisfies FeedEventsResponse);
  financeSubscriptionsMock.mockResolvedValue({
    subscriptions: [],
    count: 0,
  } satisfies FinanceSubscriptionsResponse);
  candleStreamsMock.mockResolvedValue({
    streams: [],
    count: 0,
    query_limit: 100,
    has_more: false,
  } satisfies CandleStreamsResponse);
  candlesMock.mockResolvedValue({
    candles: [],
    count: 0,
    query_limit: 50,
    has_more: false,
    next_start: null,
  } satisfies MarketCandlesResponse);
  recentCandlesMock.mockResolvedValue({
    candles: [],
    count: 0,
    query_limit: 50,
    has_more: false,
    next_end: null,
  });
  candleFreshnessMock.mockResolvedValue({
    latest: null,
    as_of: '2026-07-12T00:50:00Z',
    stale_after_secs: 120,
    lag_secs: null,
    is_stale: true,
    status: 'missing',
  } satisfies MarketCandleFreshnessResponse);
  candleGapsMock.mockResolvedValue({
    missing_open_times: [],
    missing_count: 0,
    expected_count: 50,
    complete: true,
  } satisfies MarketCandleGapsResponse);
  enableCatalogEntryMock.mockResolvedValue(feed());
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('DataFeedsPanel', () => {
  it('shows provider metadata for built-in finance catalog entries', async () => {
    catalogMock.mockResolvedValue([
      {
        id: 'binance-market-candles',
        name: 'Binance Market Candles',
        description: 'Public Binance spot OHLCV feed.',
        feed_type: 'market_candle',
        provider: 'binance',
        tags: ['finance', 'market-data', 'crypto', 'binance'],
        source_name: 'finance-binance-market-candles',
        enabled: false,
        feed_id: null,
        requires_configuration: false,
        setup_hint: null,
        transport_template: {
          provider: 'binance',
          base_url: 'https://api.binance.com',
          interval_secs: 60,
          headers: {},
          venue: 'binance',
          symbols: ['BTCUSDT', 'ETHUSDT'],
          timeframes: ['1m'],
          max_candles_per_poll: 1000,
        },
        venue: 'binance',
        configured_symbols: ['BTCUSDT', 'ETHUSDT'],
        configured_timeframes: ['1m'],
        load: {
          user_subscription_count: 0,
          subscribed_market_stream_count: 0,
          configured_market_stream_count: 2,
          configured_market_poll_request_count: 2,
          configured_market_requests_per_second: 2 / 60,
          configured_market_request_budget_per_second: 10,
          configured_market_minimum_safe_interval_secs: 5,
          configured_market_fanout_safe_to_start: true,
          configured_market_fanout_diagnostic: null,
        },
      },
      {
        id: 'longbridge-market-candles',
        name: 'Longbridge Market Data',
        description: 'Preset for Longbridge equities market data.',
        feed_type: 'market_candle',
        provider: 'longbridge',
        tags: ['finance', 'market-data', 'equities', 'longbridge'],
        source_name: 'finance-longbridge-market-candles',
        enabled: false,
        feed_id: null,
        requires_configuration: true,
        setup_hint: 'Connect Longbridge credentials behind a normalized candle endpoint.',
        transport_template: {
          url: '',
          interval_secs: 60,
          headers: {},
          venue: 'longbridge',
          symbols: ['AAPL.US', 'NVDA.US'],
          timeframes: ['1d'],
          max_candles_per_poll: 1000,
        },
        venue: 'longbridge',
        configured_symbols: ['AAPL.US', 'NVDA.US'],
        configured_timeframes: ['1d'],
      },
    ] satisfies FeedCatalogEntry[]);

    renderPanel();

    expect(await screen.findByText('Default finance sources')).toBeInTheDocument();
    expect(screen.getByText('Provider binance')).toBeInTheDocument();
    expect(screen.getByText('Provider longbridge')).toBeInTheDocument();
    expect(screen.getByText('2 configured streams · 2 req/poll · 0.03 req/s')).toBeInTheDocument();
  });

  it('warns when a catalog K-line source fan-out is unsafe', async () => {
    catalogMock.mockResolvedValue([
      {
        id: 'binance-market-candles',
        name: 'Binance Market Candles',
        description: 'Public Binance spot OHLCV feed.',
        feed_type: 'market_candle',
        provider: 'binance',
        tags: ['finance', 'market-data', 'crypto', 'binance'],
        source_name: 'finance-binance-market-candles',
        enabled: false,
        feed_id: null,
        requires_configuration: false,
        setup_hint: null,
        transport_template: {
          provider: 'binance',
          base_url: 'https://api.binance.com',
          interval_secs: 5,
          headers: {},
          venue: 'binance',
          symbols: ['BTCUSDT'],
          timeframes: ['1m'],
          max_candles_per_poll: 1000,
        },
        venue: 'binance',
        configured_symbols: ['BTCUSDT'],
        configured_timeframes: ['1m'],
        load: {
          user_subscription_count: 1,
          subscribed_market_stream_count: 100,
          configured_market_stream_count: 200,
          configured_market_poll_request_count: 200,
          configured_market_requests_per_second: 40,
          configured_market_request_budget_per_second: 10,
          configured_market_minimum_safe_interval_secs: 20,
          configured_market_fanout_safe_to_start: false,
          configured_market_fanout_diagnostic:
            'market candle watchlist fans out to 200 requests per poll at interval_secs=5; increase interval_secs to at least 20.',
        },
      },
    ] satisfies FeedCatalogEntry[]);

    renderPanel();

    expect(await screen.findByText('Unsafe fan-out')).toBeInTheDocument();
    expect(
      screen.getByText('200 configured streams · 200 req/poll · 40 req/s · 100 subscribed streams'),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        'market candle watchlist fans out to 200 requests per poll at interval_secs=5; increase interval_secs to at least 20.',
      ),
    ).toBeInTheDocument();
  });

  it('shows curated finance bundles and enables all ready sources', async () => {
    const fed: FeedCatalogEntry = {
      id: 'fed-press-releases',
      name: 'Federal Reserve Press Releases',
      description: 'Official Federal Reserve press releases.',
      feed_type: 'rss',
      provider: null,
      tags: ['finance', 'news', 'fed', 'macro'],
      source_name: 'finance-fed-press-releases',
      enabled: false,
      feed_id: null,
      requires_configuration: false,
      setup_hint: null,
      transport_template: {
        url: 'https://www.federalreserve.gov/feeds/press_all.xml',
        interval_secs: 300,
        headers: {},
        max_entries_per_poll: 50,
      },
    };
    const sec: FeedCatalogEntry = {
      id: 'sec-press-releases',
      name: 'SEC Press Releases',
      description: 'SEC press releases RSS feed.',
      feed_type: 'rss',
      provider: null,
      tags: ['finance', 'news', 'sec', 'regulatory'],
      source_name: 'finance-sec-press-releases',
      enabled: false,
      feed_id: null,
      requires_configuration: false,
      setup_hint: null,
      transport_template: {
        url: 'https://www.sec.gov/news/pressreleases.rss',
        interval_secs: 300,
        headers: {},
        max_entries_per_poll: 50,
      },
    };

    catalogMock.mockResolvedValue([fed, sec] satisfies FeedCatalogEntry[]);
    financeBundlesMock.mockResolvedValue({
      count: 1,
      bundles: [
        {
          id: 'macro-news',
          name: 'Macro News',
          description: 'Federal Reserve and SEC official RSS feeds.',
          tags: ['finance', 'news', 'macro', 'regulatory'],
          catalog_source_ids: ['fed-press-releases', 'sec-press-releases'],
          feed_types: ['rss'],
          providers: [],
          source_count: 2,
          enabled_source_count: 0,
          ready_source_count: 2,
          requires_configuration: false,
          can_enable: true,
          sources: [fed, sec],
          subscriptions: {
            user_subscribed: false,
            user_subscription_ids: [],
          },
        },
      ],
    } satisfies FinanceFeedBundlesResponse);

    renderPanel();

    expect(await screen.findByText('Curated feed bundles')).toBeInTheDocument();
    expect(screen.getByText('Macro News')).toBeInTheDocument();
    expect(screen.getByText('0/2 sources on')).toBeInTheDocument();
    expect(screen.getByText('RSS · Mixed sources')).toBeInTheDocument();
    expect(
      screen.getByText('Sources finance-fed-press-releases, finance-sec-press-releases'),
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Enable bundle' }));

    await waitFor(() => {
      expect(enableCatalogEntryMock).toHaveBeenNthCalledWith(1, 'fed-press-releases');
      expect(enableCatalogEntryMock).toHaveBeenNthCalledWith(2, 'sec-press-releases');
    });
  });

  it('discloses keyless best-effort Yahoo bundle quality', async () => {
    const yahoo: FeedCatalogEntry = {
      id: 'yahoo-market-candles',
      name: 'Yahoo US Equities Daily',
      description: 'Best-effort Yahoo Finance daily candles for research.',
      feed_type: 'market_candle',
      provider: 'yahoo',
      tags: [
        'finance',
        'market-data',
        'yahoo',
        'no-auth',
        'best-effort',
        'delayed',
        'experimental',
        'region-dependent',
        'unofficial-api',
      ],
      source_name: 'finance-yahoo-market-candles',
      enabled: false,
      feed_id: null,
      requires_configuration: false,
      setup_hint: null,
      transport_template: {
        provider: 'yahoo',
        venue: 'yahoo',
        symbols: ['AAPL'],
        timeframes: ['1d'],
      },
    };

    catalogMock.mockResolvedValue([yahoo] satisfies FeedCatalogEntry[]);
    financeBundlesMock.mockResolvedValue({
      count: 1,
      bundles: [
        {
          id: 'yahoo-us-equities-daily',
          name: 'Yahoo US Equities Daily',
          description: 'Keyless best-effort daily candles.',
          tags: ['finance', 'market-data', 'equities', 'yahoo', 'best-effort'],
          catalog_source_ids: ['yahoo-market-candles'],
          feed_types: ['market_candle'],
          providers: ['yahoo'],
          source_count: 1,
          enabled_source_count: 0,
          ready_source_count: 1,
          requires_configuration: false,
          can_enable: true,
          sources: [yahoo],
          subscriptions: {
            user_subscribed: false,
            user_subscription_ids: [],
          },
        },
      ],
    } satisfies FinanceFeedBundlesResponse);

    renderPanel();

    expect(await screen.findAllByText('No key required')).toHaveLength(2);
    expect(screen.getAllByText('Best effort')).toHaveLength(2);
    expect(screen.getAllByText('Delayed / unofficial')).toHaveLength(2);
    expect(screen.getAllByText('Region dependent')).toHaveLength(2);
  });

  it('opens the catalog configure dialog from a requires-config finance bundle', async () => {
    const longbridge: FeedCatalogEntry = {
      id: 'longbridge-market-candles',
      name: 'Longbridge Market Data',
      description: 'Preset for Longbridge equities market data.',
      feed_type: 'market_candle',
      provider: 'longbridge',
      tags: ['finance', 'market-data', 'equities', 'longbridge'],
      source_name: 'finance-longbridge-market-candles',
      enabled: false,
      feed_id: null,
      requires_configuration: true,
      setup_hint: 'Connect Longbridge credentials behind a normalized candle endpoint.',
      transport_template: {
        url: '',
        interval_secs: 60,
        headers: {},
        venue: 'longbridge',
        symbols: [],
        timeframes: [],
        max_candles_per_poll: 1000,
      },
    };

    catalogMock.mockResolvedValue([longbridge] satisfies FeedCatalogEntry[]);
    financeBundlesMock.mockResolvedValue({
      count: 1,
      bundles: [
        {
          id: 'longbridge-equities-daily',
          name: 'Longbridge Equities Daily',
          description: 'Longbridge equities daily candles preset.',
          tags: ['finance', 'market-data', 'equities', 'longbridge'],
          catalog_source_ids: ['longbridge-market-candles'],
          feed_types: ['market_candle'],
          providers: ['longbridge'],
          source_count: 1,
          enabled_source_count: 0,
          ready_source_count: 0,
          requires_configuration: true,
          can_enable: false,
          sources: [longbridge],
          subscriptions: {
            user_subscribed: false,
            user_subscription_ids: [],
          },
        },
      ],
    } satisfies FinanceFeedBundlesResponse);

    renderPanel();

    expect(await screen.findByText('Longbridge Equities Daily')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Configure source' }));

    expect(await screen.findByText('Configure Longbridge Market Data')).toBeInTheDocument();
    fireEvent.change(screen.getByPlaceholderText('https://market-data.example/candles/latest'), {
      target: { value: 'https://market-data.local/longbridge/candles/latest' },
    });
    fireEvent.change(screen.getByPlaceholderText('BTCUSDT, ETHUSDT'), {
      target: { value: 'AAPL.US, NVDA.US' },
    });
    fireEvent.change(screen.getByPlaceholderText('1m, 15m, 1h'), {
      target: { value: '1d' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Enable source' }));

    await waitFor(() => {
      expect(enableCatalogEntryMock).toHaveBeenCalledWith('longbridge-market-candles', {
        transport: expect.objectContaining({
          url: 'https://market-data.local/longbridge/candles/latest',
          venue: 'longbridge',
          symbols: ['AAPL.US', 'NVDA.US'],
          timeframes: ['1d'],
        }),
        auth: null,
      });
    });
  });

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
      has_more: false,
    } satisfies CandleStreamsResponse);

    renderPanel();

    expect(await screen.findByText('Stored K-line streams')).toBeInTheDocument();
    expect(await screen.findByText('BINANCE · BTCUSDT · 1m')).toBeInTheDocument();
    expect(screen.getAllByText('finance-binance-market-candles').length).toBeGreaterThan(0);
    expect(screen.getByText('42')).toBeInTheDocument();

    await waitFor(() => {
      expect(candleStreamsMock).toHaveBeenCalledWith({ limit: 500 });
    });
  });

  it('shows the last feed event type from summary data', async () => {
    listMock.mockResolvedValue([feed()]);
    summariesMock.mockResolvedValue([
      {
        feed_id: 'feed-1',
        source_name: 'finance-binance-market-candles',
        event_count: 12,
        last_event_type: 'market_candle_closed',
        last_event_at: '2026-07-12T00:42:02Z',
        lag_seconds: 30,
      },
    ]);

    renderPanel();

    expect(await screen.findByText('market_candle_closed')).toBeInTheDocument();
    expect(screen.getByText('30s lag')).toBeInTheDocument();
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

  it('warns when the stored K-line stream overview reaches the query limit', async () => {
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

    renderPanel();

    expect(await screen.findByText('Stored K-line streams')).toBeInTheDocument();
    expect(
      screen.getByText(
        'Showing the first 1 streams. Narrow by source, venue, symbol, or timeframe if a watched K-line stream is missing from this overview.',
      ),
    ).toBeInTheDocument();
  });

  it('applies stored K-line stream filters before querying watermarks', async () => {
    renderPanel();

    expect(await screen.findByText('Stored K-line streams')).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('K-line source'), {
      target: { value: 'finance-binance-market-candles' },
    });
    fireEvent.change(screen.getByLabelText('K-line venue'), {
      target: { value: 'binance' },
    });
    fireEvent.change(screen.getByLabelText('K-line symbol'), {
      target: { value: 'ETHUSDT' },
    });
    fireEvent.change(screen.getByLabelText('K-line timeframe'), {
      target: { value: '1m' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Apply filters' }));

    await waitFor(() => {
      expect(candleStreamsMock).toHaveBeenCalledWith({
        source_name: 'finance-binance-market-candles',
        venue: 'binance',
        symbol: 'ETHUSDT',
        timeframe: '1m',
        limit: 500,
      });
    });
  });

  it('clears stored K-line stream filters back to the default overview', async () => {
    renderPanel();

    expect(await screen.findByText('Stored K-line streams')).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('K-line venue'), {
      target: { value: 'binance' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Apply filters' }));

    await waitFor(() => {
      expect(candleStreamsMock).toHaveBeenCalledWith({ venue: 'binance', limit: 500 });
    });

    fireEvent.click(screen.getByRole('button', { name: 'Clear filters' }));

    await waitFor(() => {
      expect(candleStreamsMock).toHaveBeenLastCalledWith({ limit: 500 });
    });
  });

  it('filters feed history by backend event kind', async () => {
    listMock.mockResolvedValue([feed()]);

    renderPanel();

    fireEvent.click(await screen.findByRole('button', { name: 'finance-binance-market-candles' }));
    await waitFor(() => {
      expect(eventsMock).toHaveBeenLastCalledWith('feed-1', {
        since: '24h',
        limit: 50,
        offset: 0,
      });
    });

    fireEvent.pointerDown(screen.getByRole('combobox', { name: 'Event type' }), {
      button: 0,
      ctrlKey: false,
      pointerType: 'mouse',
    });
    fireEvent.click(await screen.findByRole('option', { name: 'K-line closed' }));

    await waitFor(() => {
      expect(eventsMock).toHaveBeenLastCalledWith('feed-1', {
        since: '24h',
        event_kinds: ['market_candle_closed'],
        limit: 50,
        offset: 0,
      });
    });
  });

  it('shows a compact K-line summary in feed history rows', async () => {
    listMock.mockResolvedValue([feed()]);
    eventsMock.mockResolvedValue({
      events: [
        {
          id: 'event-1',
          source_name: 'finance-binance-market-candles',
          event_type: 'market_candle_closed',
          tags: ['finance', 'market-data', 'venue:binance', 'symbol:BTCUSDT', 'timeframe:1m'],
          payload: {
            venue: 'binance',
            symbol: 'BTCUSDT',
            timeframe: '1m',
            close_time: '2026-07-12T00:42:00Z',
            close: '61610.30',
          },
          received_at: '2026-07-12T00:42:02Z',
        },
      ],
      total: 1,
      has_more: false,
    } satisfies FeedEventsResponse);

    renderPanel();

    fireEvent.click(await screen.findByRole('button', { name: 'finance-binance-market-candles' }));

    expect(await screen.findByText('BINANCE · BTCUSDT · 1m')).toBeInTheDocument();
    expect(screen.getByText('close 61610.30')).toBeInTheDocument();
  });

  it('shows fresh health for a K-line subscription with a matching stored stream', async () => {
    vi.spyOn(Date, 'now').mockReturnValue(Date.parse('2026-07-12T00:42:30Z'));
    financeSubscriptionsMock.mockResolvedValue({
      subscriptions: [candleSubscription()],
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

    renderPanel();

    expect(await screen.findByText('K-line Fresh')).toBeInTheDocument();
    expect(screen.getByText('1/1 matched stream fresh.')).toBeInTheDocument();
  });

  it('shows missing health before a K-line subscription has stored candles', async () => {
    financeSubscriptionsMock.mockResolvedValue({
      subscriptions: [candleSubscription({ symbols: ['SOLUSDT'] })],
      count: 1,
    } satisfies FinanceSubscriptionsResponse);

    renderPanel();

    expect(await screen.findByText('K-line Missing')).toBeInTheDocument();
    expect(
      screen.getByText('No stored K-line stream matches this subscription yet.'),
    ).toBeInTheDocument();
  });

  it('loads a recent candle preview for a stored K-line stream', async () => {
    candleStreamsMock.mockResolvedValue({
      streams: [
        {
          source_name: 'finance-binance-market-candles',
          venue: 'binance',
          symbol: 'BTCUSDT',
          timeframe: '1m',
          candle_count: 50,
          first_open_time: '2026-07-12T00:00:00Z',
          latest_open_time: '2026-07-12T00:49:00Z',
          latest_close_time: '2026-07-12T00:49:59Z',
          latest_ingested_at: '2026-07-12T00:50:02Z',
        },
      ],
      count: 1,
      query_limit: 100,
      has_more: false,
    } satisfies CandleStreamsResponse);
    recentCandlesMock.mockResolvedValue({
      candles: [
        {
          source_name: 'finance-binance-market-candles',
          venue: 'binance',
          symbol: 'BTCUSDT',
          timeframe: '1m',
          open_time: '2026-07-12T00:48:00Z',
          close_time: '2026-07-12T00:48:59Z',
          open: '64100.10',
          high: '64120.00',
          low: '64090.00',
          close: '64110.50',
          volume: '12.34',
          ingested_at: '2026-07-12T00:49:02Z',
          provider_sequence: null,
        },
        {
          source_name: 'finance-binance-market-candles',
          venue: 'binance',
          symbol: 'BTCUSDT',
          timeframe: '1m',
          open_time: '2026-07-12T00:49:00Z',
          close_time: '2026-07-12T00:49:59Z',
          open: '64110.50',
          high: '64150.00',
          low: '64105.00',
          close: '64140.25',
          volume: '8.75',
          ingested_at: '2026-07-12T00:50:02Z',
          provider_sequence: null,
        },
      ],
      count: 2,
      query_limit: 50,
      has_more: false,
      next_end: null,
    });
    candleFreshnessMock.mockResolvedValue({
      latest: {
        source_name: 'finance-binance-market-candles',
        venue: 'binance',
        symbol: 'BTCUSDT',
        timeframe: '1m',
        open_time: '2026-07-12T00:49:00Z',
        close_time: '2026-07-12T00:49:59Z',
        open: '64110.50',
        high: '64150.00',
        low: '64105.00',
        close: '64140.25',
        volume: '8.75',
        ingested_at: '2026-07-12T00:50:02Z',
        provider_sequence: null,
      },
      as_of: '2026-07-12T00:50:30Z',
      stale_after_secs: 120,
      lag_secs: 31,
      is_stale: false,
      status: 'fresh',
    } satisfies MarketCandleFreshnessResponse);
    candleGapsMock.mockResolvedValue({
      missing_open_times: [],
      missing_count: 0,
      expected_count: 50,
      complete: true,
    } satisfies MarketCandleGapsResponse);

    renderPanel();

    const previewButton = await screen.findByRole('button', {
      name: 'Preview BINANCE · BTCUSDT · 1m',
    });
    fireEvent.click(previewButton);

    await waitFor(() => {
      expect(recentCandlesMock).toHaveBeenCalledWith({
        source_name: 'finance-binance-market-candles',
        venue: 'binance',
        symbol: 'BTCUSDT',
        timeframe: '1m',
        limit: 50,
      });
    });
    await waitFor(() => {
      expect(candleFreshnessMock).toHaveBeenCalledWith({
        source_name: 'finance-binance-market-candles',
        venue: 'binance',
        symbol: 'BTCUSDT',
        timeframe: '1m',
      });
      expect(candleGapsMock).toHaveBeenCalledWith({
        source_name: 'finance-binance-market-candles',
        venue: 'binance',
        symbol: 'BTCUSDT',
        timeframe: '1m',
        start: '2026-07-12T00:00:00.000Z',
        end: '2026-07-12T00:50:00.000Z',
      });
    });
    expect(await screen.findByText('Recent candles · BINANCE · BTCUSDT · 1m')).toBeInTheDocument();
    expect(screen.getByText('31s lag · stale after 120s')).toBeInTheDocument();
    expect(screen.getByText('0/50 missing in preview window')).toBeInTheDocument();
    expect(screen.getAllByText('64140.25').length).toBeGreaterThan(0);
    expect(screen.getByText('12.34')).toBeInTheDocument();
  });

  it('warns when the candle preview reaches the query limit', async () => {
    candleStreamsMock.mockResolvedValue({
      streams: [
        {
          source_name: 'finance-binance-market-candles',
          venue: 'binance',
          symbol: 'BTCUSDT',
          timeframe: '1m',
          candle_count: 75,
          first_open_time: '2026-07-12T00:00:00Z',
          latest_open_time: '2026-07-12T01:14:00Z',
          latest_close_time: '2026-07-12T01:14:59Z',
          latest_ingested_at: '2026-07-12T01:15:02Z',
        },
      ],
      count: 1,
      query_limit: 100,
      has_more: false,
    } satisfies CandleStreamsResponse);
    recentCandlesMock.mockResolvedValue({
      candles: Array.from({ length: 50 }, (_, index) => ({
        source_name: 'finance-binance-market-candles',
        venue: 'binance',
        symbol: 'BTCUSDT',
        timeframe: '1m',
        open_time: new Date(Date.UTC(2026, 6, 12, 0, index)).toISOString(),
        close_time: new Date(Date.UTC(2026, 6, 12, 0, index, 59)).toISOString(),
        open: '64100.00',
        high: '64150.00',
        low: '64090.00',
        close: '64125.00',
        volume: '10.00',
        ingested_at: new Date(Date.UTC(2026, 6, 12, 0, index, 59)).toISOString(),
        provider_sequence: null,
      })),
      count: 50,
      query_limit: 50,
      has_more: true,
      next_end: '2026-07-12T00:00:00Z',
    });

    renderPanel();

    const previewButton = await screen.findByRole('button', {
      name: 'Preview BINANCE · BTCUSDT · 1m',
    });
    fireEvent.click(previewButton);

    expect(
      await screen.findByText(/Showing the latest 50 candles in this preview/),
    ).toBeInTheDocument();
    expect(screen.getByText(/Older page ends before/)).toBeInTheDocument();
    expect(screen.getByText('2026-07-12T00:00:00Z')).toBeInTheDocument();
  });
});
