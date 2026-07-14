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

import { test, expect } from '@playwright/test';

// ---------------------------------------------------------------------------
// Types — mirrors web/src/api/data-feeds.ts
// ---------------------------------------------------------------------------

interface DataFeedConfig {
  id: string;
  name: string;
  feed_type: 'webhook' | 'websocket' | 'polling' | 'rss' | 'market_candle';
  tags: string[];
  transport: Record<string, unknown>;
  auth: { type: string; [key: string]: unknown } | null;
  enabled: boolean;
  status: 'idle' | 'running' | 'error';
  last_error: string | null;
  created_at: string;
  updated_at: string;
}

interface FeedEvent {
  id: string;
  source_name: string;
  event_type: string;
  tags: string[];
  payload: unknown;
  received_at: string;
}

interface FeedSummary {
  feed_id: string;
  source_name: string;
  event_count: number;
  last_event_at: string | null;
  lag_seconds: number | null;
}

interface FeedCatalogEntry {
  id: string;
  name: string;
  description: string;
  feed_type: DataFeedConfig['feed_type'];
  provider?: string | null;
  tags: string[];
  source_name?: string;
  enabled: boolean;
  feed_id: string | null;
  requires_configuration: boolean;
  setup_hint: string | null;
  transport_template: Record<string, unknown> | null;
  venue?: string | null;
  configured_symbols?: string[];
  configured_timeframes?: string[];
  subscriptions?: {
    user_subscribed: boolean;
    user_subscription_ids: string[];
  };
}

interface FinanceSubscriptionSource {
  source_name: string;
  catalog_source_id: string | null;
  catalog_name: string | null;
  provider: string | null;
  feed_id: string | null;
  feed_type: DataFeedConfig['feed_type'] | null;
  enabled: boolean | null;
  status: DataFeedConfig['status'] | null;
}

interface FinanceSubscription {
  subscription_id: string;
  session_key: string;
  event_kinds: Array<'rss_article' | 'market_candle_closed'>;
  source_names: string[];
  matches_all_sources: boolean;
  sources: FinanceSubscriptionSource[];
  category_tags: string[];
  watch_terms: string[];
  venues: string[];
  symbols: string[];
  timeframes: string[];
  delivery: 'immediate' | 'silent';
  cooldown_secs: number;
  max_immediate_per_hour: number;
}

interface FinanceSubscriptionsResponse {
  subscriptions: FinanceSubscription[];
  count: number;
}

interface FinanceFeedBundle {
  id: string;
  name: string;
  description: string;
  tags: string[];
  catalog_source_ids: string[];
  feed_types: DataFeedConfig['feed_type'][];
  providers: string[];
  source_count: number;
  enabled_source_count: number;
  ready_source_count: number;
  requires_configuration: boolean;
  can_enable: boolean;
  sources: FeedCatalogEntry[];
  subscriptions: {
    user_subscribed: boolean;
    user_subscription_ids: string[];
  };
}

interface FinanceFeedBundlesResponse {
  bundles: FinanceFeedBundle[];
  count: number;
}

interface ChatSession {
  key: string;
  title: string | null;
  model: string | null;
  model_provider: string | null;
  thinking_level: string | null;
  system_prompt: string | null;
  message_count: number;
  preview: string | null;
  anchors: unknown[];
  status: 'active' | 'archived';
  metadata: Record<string, unknown> | null;
  created_at: string;
  updated_at: string;
}

// ---------------------------------------------------------------------------
// Shared mock data
// ---------------------------------------------------------------------------

/** Yahoo Finance payload — fetched once in beforeAll, or falls back to static fixture. */
let yahooPayload: unknown;

const YAHOO_API_URL = 'https://query1.finance.yahoo.com/v8/finance/chart/AAPL?interval=1d&range=1d';

/** Static fallback when Yahoo Finance API is unreachable. */
const YAHOO_FALLBACK = {
  chart: {
    result: [
      {
        meta: {
          currency: 'USD',
          symbol: 'AAPL',
          exchangeName: 'NMS',
          fullExchangeName: 'NasdaqGS',
          regularMarketPrice: 195.89,
        },
        timestamp: [1713100200],
        indicators: {
          quote: [
            {
              open: [194.5],
              high: [196.12],
              low: [193.87],
              close: [195.89],
              volume: [54_321_000],
            },
          ],
        },
      },
    ],
    error: null,
  },
};

/** Fake settings that satisfy hasConfiguredLlmProvider so onboarding is skipped. */
const MOCK_SETTINGS: Record<string, string> = {
  'llm.default_provider': 'openrouter',
  'llm.providers.openrouter.enabled': 'true',
  'llm.providers.openrouter.api_key': 'sk-fake-key-for-e2e',
};

function makeFeed(overrides: Partial<DataFeedConfig> = {}): DataFeedConfig {
  const now = new Date().toISOString();
  return {
    id: 'feed-1',
    name: 'yahoo-aapl',
    feed_type: 'polling',
    tags: ['stock', 'yahoo', 'aapl'],
    transport: {
      url: YAHOO_API_URL,
      interval_secs: 60,
      headers: {},
      method: 'GET',
    },
    auth: null,
    enabled: true,
    status: 'running',
    last_error: null,
    created_at: now,
    updated_at: now,
    ...overrides,
  };
}

function makeEvent(overrides: Partial<FeedEvent> = {}): FeedEvent {
  return {
    id: 'evt-1',
    source_name: 'yahoo-aapl',
    event_type: 'poll_response',
    tags: ['stock', 'yahoo', 'aapl'],
    payload: yahooPayload,
    received_at: new Date().toISOString(),
    ...overrides,
  };
}

function makeSummary(
  feed: DataFeedConfig,
  events: FeedEvent[],
  overrides: Partial<FeedSummary> = {},
): FeedSummary {
  const matchingEvents = events.filter((event) => event.source_name === feed.name);
  const lastEventAt =
    matchingEvents
      .map((event) => event.received_at)
      .sort()
      .at(-1) ?? null;

  return {
    feed_id: feed.id,
    source_name: feed.name,
    event_count: matchingEvents.length,
    last_event_at: lastEventAt,
    lag_seconds: lastEventAt
      ? Math.max(0, Math.floor((Date.now() - Date.parse(lastEventAt)) / 1000))
      : null,
    ...overrides,
  };
}

function makeChatSession(overrides: Partial<ChatSession> = {}): ChatSession {
  const now = new Date().toISOString();
  return {
    key: 'session-1',
    title: 'Finance research',
    model: null,
    model_provider: null,
    thinking_level: null,
    system_prompt: null,
    message_count: 0,
    preview: null,
    anchors: [],
    status: 'active',
    metadata: null,
    created_at: now,
    updated_at: now,
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// Setup — fetch real Yahoo Finance data once for realistic payloads
// ---------------------------------------------------------------------------

test.beforeAll(async () => {
  try {
    const res = await fetch(YAHOO_API_URL, {
      headers: { 'User-Agent': 'Mozilla/5.0' },
      signal: AbortSignal.timeout(5_000),
    });
    if (res.ok) {
      yahooPayload = await res.json();
    } else {
      yahooPayload = YAHOO_FALLBACK;
    }
  } catch {
    yahooPayload = YAHOO_FALLBACK;
  }
});

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * Install API route mocks and suppress UI dialogs that block interaction.
 *
 * - Sets localStorage entries to prevent the ConnectionSetupDialog (needs
 *   rara_backend_url) and OnboardingModal (needs onboarding_dismissed).
 * - Mocks /health, /settings, and all /data-feeds endpoints.
 *
 * IMPORTANT: The rara_backend_url is set to the Vite dev server origin so
 * that resolveUrl() produces absolute URLs pointing back at the same origin.
 * Playwright's page.route() then intercepts these before they hit the network.
 */
async function setupRoutes(
  page: import('@playwright/test').Page,
  state: {
    feeds: DataFeedConfig[];
    events: FeedEvent[];
    summaries?: FeedSummary[];
    catalog?: FeedCatalogEntry[];
    financeBundles?: FinanceFeedBundlesResponse;
    financeSubscriptions?: FinanceSubscriptionsResponse;
    sessions?: ChatSession[];
    lastCatalogEnableBody?: unknown;
  },
) {
  // Suppress onboarding & connection dialogs via localStorage.
  await page.addInitScript(() => {
    // Point resolveUrl() at the page origin (vite preview or dev server)
    // so fetches stay same-origin and page.route can intercept them.
    // hasCustomBackendUrl() needs a non-null value to suppress
    // ConnectionSetupDialog.
    localStorage.setItem('rara_backend_url', location.origin);
    localStorage.setItem('onboarding_dismissed', 'true');
    // Satisfy the RequireAuth route guard (owner-token login): it reads
    // both keys straight from localStorage, no network round-trip.
    localStorage.setItem('access_token', 'e2e-fake-token');
    localStorage.setItem(
      'auth_user',
      JSON.stringify({ user_id: 'owner', role: 'owner', is_admin: true }),
    );
  });

  // Health check.
  await page.route('**/api/v1/health', (route) =>
    route.fulfill({ status: 200, json: { status: 'ok' } }),
  );

  // Settings — return a configured provider so onboarding is suppressed.
  await page.route('**/api/v1/settings', (route) =>
    route.fulfill({ status: 200, json: MOCK_SETTINGS }),
  );

  const sessions = state.sessions ?? [];

  await page.route(/\/api\/v1\/chat\/sessions(\?.*)?$/, async (route, request) => {
    if (request.method() === 'GET') {
      await route.fulfill({ json: sessions });
      return;
    }

    if (request.method() === 'POST') {
      const created = makeChatSession({
        key: `session-${Date.now()}`,
        title: 'New session',
      });
      sessions.unshift(created);
      await route.fulfill({ json: created });
      return;
    }

    await route.continue();
  });

  await page.route(/\/api\/v1\/chat\/sessions\/([^/?]+)\/messages(\?.*)?$/, async (route) => {
    await route.fulfill({ json: [] });
  });

  await page.route(/\/api\/v1\/chat\/sessions\/([^/?]+)(\?.*)?$/, async (route, request) => {
    if (request.method() !== 'GET') {
      await route.continue();
      return;
    }

    const key = decodeURIComponent(request.url().match(/chat\/sessions\/([^/?]+)/)?.[1] ?? '');
    const session = sessions.find((item) => item.key === key);
    if (!session) {
      await route.fulfill({ status: 404, json: { error: 'not found' } });
      return;
    }
    await route.fulfill({ json: session });
  });

  // Default feed source catalog.
  await page.route('**/api/v1/data-feeds/catalog', async (route) => {
    await route.fulfill({ json: state.catalog ?? [] });
  });

  await page.route(/\/api\/v1\/data-feeds\/finance\/bundles(\?.*)?$/, async (route) => {
    await route.fulfill({ json: state.financeBundles ?? { bundles: [], count: 0 } });
  });

  await page.route('**/api/v1/data-feeds/summary', async (route) => {
    await route.fulfill({
      json: state.summaries ?? state.feeds.map((feed) => makeSummary(feed, state.events)),
    });
  });

  await page.route('**/api/v1/data-feeds/finance/subscriptions', async (route, request) => {
    if (!state.financeSubscriptions) {
      await route.continue();
      return;
    }

    if (request.method() === 'GET') {
      await route.fulfill({ json: state.financeSubscriptions });
      return;
    }

    if (request.method() === 'POST') {
      const body = request.postDataJSON() as {
        session_key: string;
        catalog_source_ids?: string[];
        delivery?: 'immediate' | 'silent';
        venues?: string[];
        symbols?: string[];
        timeframes?: string[];
      };
      const entries = (body.catalog_source_ids ?? [])
        .map((id) => state.catalog?.find((item) => item.id === id))
        .filter((entry): entry is FeedCatalogEntry => entry != null);
      const eventKinds = Array.from(
        new Set(
          entries.map((entry) =>
            entry.feed_type === 'market_candle' ? 'market_candle_closed' : 'rss_article',
          ),
        ),
      ) as FinanceSubscription['event_kinds'];
      const subscription: FinanceSubscription = {
        subscription_id: `sub-${Date.now()}`,
        session_key: body.session_key,
        event_kinds: eventKinds.length > 0 ? eventKinds : ['rss_article'],
        source_names:
          entries.length > 0
            ? entries.map((entry) => entry.source_name ?? `finance-${entry.id}`)
            : ['finance-unknown'],
        matches_all_sources: false,
        sources: entries.map((entry) => ({
          source_name: entry.source_name ?? `finance-${entry.id}`,
          catalog_source_id: entry.id,
          catalog_name: entry.name,
          provider: entry.provider ?? null,
          feed_id: entry.feed_id,
          feed_type: entry.feed_type,
          enabled: entry.enabled,
          status: entry.enabled ? 'running' : 'idle',
        })),
        category_tags: [],
        watch_terms: [],
        venues: body.venues ?? [],
        symbols: body.symbols ?? [],
        timeframes: body.timeframes ?? [],
        delivery: body.delivery ?? 'silent',
        cooldown_secs: 900,
        max_immediate_per_hour: 6,
      };
      const response = state.financeSubscriptions ?? { subscriptions: [], count: 0 };
      response.subscriptions.push(subscription);
      response.count = response.subscriptions.length;
      state.financeSubscriptions = response;
      await route.fulfill({ status: 201, json: { subscription, created: true } });
      return;
    }

    await route.continue();
  });

  await page.route('**/api/v1/data-feeds/catalog/*/enable', async (route, request) => {
    if (request.method() !== 'POST') {
      await route.continue();
      return;
    }

    const id = request.url().match(/catalog\/([^/]+)\/enable/)?.[1];
    const entry = state.catalog?.find((item) => item.id === id);
    if (!entry) {
      await route.fulfill({ status: 404, json: { error: 'not found' } });
      return;
    }

    const body = request.postData()
      ? (request.postDataJSON() as {
          transport?: Record<string, unknown>;
          auth?: { type: string; [key: string]: unknown } | null;
        })
      : {};
    state.lastCatalogEnableBody = body;

    const now = new Date().toISOString();
    const feed: DataFeedConfig = {
      id: entry.feed_id ?? `feed-${entry.id}`,
      name: `finance-${entry.id}`,
      feed_type: entry.feed_type,
      tags: entry.tags,
      transport: { ...(entry.transport_template ?? {}), ...(body.transport ?? {}) },
      auth: body.auth ?? null,
      enabled: true,
      status: 'running',
      last_error: null,
      created_at: now,
      updated_at: now,
    };
    state.feeds.push(feed);
    entry.enabled = true;
    entry.feed_id = feed.id;
    for (const bundle of state.financeBundles?.bundles ?? []) {
      const source = bundle.sources.find((item) => item.id === entry.id);
      if (!source) continue;
      source.enabled = true;
      source.feed_id = feed.id;
      bundle.enabled_source_count = bundle.sources.filter((item) => item.enabled).length;
    }
    await route.fulfill({ status: 201, json: feed });
  });

  await page.route('**/api/v1/data-feeds/catalog/*/disable', async (route, request) => {
    if (request.method() !== 'POST') {
      await route.continue();
      return;
    }

    const id = request.url().match(/catalog\/([^/]+)\/disable/)?.[1];
    const entry = state.catalog?.find((item) => item.id === id);
    const feed = state.feeds.find((item) => item.id === entry?.feed_id);
    if (!entry || !feed) {
      await route.fulfill({ status: 404, json: { error: 'not found' } });
      return;
    }

    feed.enabled = false;
    feed.status = 'idle';
    entry.enabled = false;
    await route.fulfill({ json: feed });
  });

  // Data feeds list + create.
  await page.route('**/api/v1/data-feeds', async (route, request) => {
    const method = request.method();
    if (method === 'GET') {
      await route.fulfill({ json: state.feeds });
    } else if (method === 'POST') {
      const body = request.postDataJSON();
      const now = new Date().toISOString();
      const created: DataFeedConfig = {
        id: `feed-${Date.now()}`,
        name: body.name,
        feed_type: body.feed_type,
        tags: body.tags ?? [],
        transport: body.transport ?? {},
        auth: body.auth ?? null,
        enabled: true,
        status: 'running',
        last_error: null,
        created_at: now,
        updated_at: now,
      };
      state.feeds.push(created);
      await route.fulfill({ json: created });
    } else {
      await route.continue();
    }
  });

  // Toggle feed.
  await page.route('**/api/v1/data-feeds/*/toggle', async (route, request) => {
    if (request.method() === 'PUT') {
      const url = request.url();
      const idMatch = url.match(/data-feeds\/([^/]+)\/toggle/);
      const id = idMatch?.[1];
      const feed = state.feeds.find((f) => f.id === id);
      if (feed) {
        feed.enabled = !feed.enabled;
        feed.status = feed.enabled ? 'running' : 'idle';
        await route.fulfill({ json: feed });
      } else {
        await route.fulfill({ status: 404, json: { error: 'not found' } });
      }
    } else {
      await route.continue();
    }
  });

  // Feed events — must be registered before the single-feed catch-all.
  await page.route('**/api/v1/data-feeds/*/events*', async (route) => {
    await route.fulfill({
      json: {
        events: state.events,
        total: state.events.length,
        has_more: false,
      },
    });
  });

  // Single feed operations (GET/PUT/DELETE by id).
  await page.route(/\/api\/v1\/data-feeds\/(?!catalog$|summary$)[^/]+$/, async (route, request) => {
    const method = request.method();
    const url = request.url();
    const idMatch = url.match(/data-feeds\/([^/]+)$/);
    const id = idMatch?.[1];

    if (method === 'DELETE') {
      state.feeds = state.feeds.filter((f) => f.id !== id);
      await route.fulfill({ status: 204 });
    } else if (method === 'PUT') {
      const feed = state.feeds.find((f) => f.id === id);
      if (feed) {
        const body = request.postDataJSON();
        Object.assign(feed, body, { updated_at: new Date().toISOString() });
        await route.fulfill({ json: feed });
      } else {
        await route.fulfill({ status: 404, json: { error: 'not found' } });
      }
    } else if (method === 'GET') {
      const feed = state.feeds.find((f) => f.id === id);
      if (feed) {
        await route.fulfill({ json: feed });
      } else {
        await route.fulfill({ status: 404, json: { error: 'not found' } });
      }
    } else {
      await route.continue();
    }
  });
}

/** Navigate directly to the Data Feeds settings tab. */
async function goToDataFeeds(page: import('@playwright/test').Page) {
  await page.goto('/settings?section=data-feeds');
  await page.waitForLoadState('networkidle');
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

test.describe('Data Feeds Management', () => {
  // -----------------------------------------------------------------------
  // 1. Navigate to Data Feeds tab
  // -----------------------------------------------------------------------

  test('navigate to Data Feeds tab in settings', async ({ page }) => {
    await setupRoutes(page, { feeds: [], events: [] });
    await page.goto('/settings');

    // Click the Data Feeds sidebar button.
    const navButton = page.locator('button', { hasText: 'Data Feeds' });
    await expect(navButton).toBeVisible({ timeout: 10_000 });
    await navButton.click();

    // The Data Feeds panel should render.
    await expect(page.getByText('External data sources that push events into rara.')).toBeVisible();
  });

  // -----------------------------------------------------------------------
  // 2. Empty state
  // -----------------------------------------------------------------------

  test('shows empty state when no feeds configured', async ({ page }) => {
    await setupRoutes(page, { feeds: [], events: [] });
    await goToDataFeeds(page);

    await expect(page.getByText('No data feeds configured')).toBeVisible({ timeout: 10_000 });
    await expect(page.getByText('Create Feed')).toBeVisible();
  });

  test('shows default finance sources and enables one', async ({ page }) => {
    await setupRoutes(page, {
      feeds: [],
      events: [],
      catalog: [
        {
          id: 'fed-press-releases',
          name: 'Federal Reserve Press Releases',
          description: 'Official Federal Reserve press releases.',
          feed_type: 'rss',
          tags: ['finance', 'news', 'fed'],
          source_name: 'finance-fed-press-releases',
          enabled: false,
          feed_id: null,
          requires_configuration: false,
          setup_hint: null,
          subscriptions: {
            user_subscribed: true,
            user_subscription_ids: ['sub-fed'],
          },
          transport_template: {
            url: 'https://www.federalreserve.gov/feeds/press_all.xml',
            interval_secs: 300,
            headers: {},
            max_entries_per_poll: 50,
          },
        },
        {
          id: 'binance-market-candles',
          name: 'Binance Market Candles',
          description: 'Public Binance spot OHLCV feed.',
          feed_type: 'market_candle',
          provider: 'binance',
          tags: ['finance', 'market-data', 'crypto', 'binance'],
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
        },
      ],
    });
    await goToDataFeeds(page);

    await expect(page.getByText('Default finance sources')).toBeVisible({ timeout: 10_000 });
    await expect(page.getByText('News feeds', { exact: true })).toBeVisible();
    await expect(page.getByText('K-line feeds', { exact: true })).toBeVisible();
    await expect(page.getByText('Federal Reserve Press Releases', { exact: true })).toBeVisible();
    await expect(page.getByText('Source finance-fed-press-releases')).toBeVisible();
    await expect(page.getByText('Subscribed')).toBeVisible();
    await expect(page.getByText('Provider binance')).toBeVisible();
    await expect(page.getByText('binance · BTCUSDT, ETHUSDT · 1m')).toBeVisible();

    const fedEntry = page
      .getByText('Federal Reserve Press Releases', { exact: true })
      .locator('xpath=ancestor::div[contains(@class, "justify-between")][1]');
    await fedEntry.getByRole('button', { name: 'Enable' }).click();

    await expect(page.getByRole('button', { name: 'finance-fed-press-releases' })).toBeVisible({
      timeout: 5_000,
    });
    await expect(page.getByRole('button', { name: 'Disable' })).toBeVisible();
  });

  test('shows curated finance bundles in settings and enables ready sources', async ({ page }) => {
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
    const state = {
      feeds: [] as DataFeedConfig[],
      events: [] as FeedEvent[],
      catalog: [fed, sec],
      financeSubscriptions: { subscriptions: [], count: 0 },
      financeBundles: {
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
      },
    };
    await setupRoutes(page, state);
    await goToDataFeeds(page);

    await expect(page.getByText('Curated feed bundles')).toBeVisible({ timeout: 10_000 });
    await expect(page.getByText('Macro News', { exact: true })).toBeVisible();
    await expect(page.getByText('0/2 sources on')).toBeVisible();
    await expect(
      page.getByText('Sources finance-fed-press-releases, finance-sec-press-releases'),
    ).toBeVisible();

    await page.getByRole('button', { name: 'Enable bundle' }).click();

    await expect(page.getByText('2/2 sources on')).toBeVisible({ timeout: 10_000 });
    expect(state.feeds.map((feed) => feed.name).sort()).toEqual([
      'finance-fed-press-releases',
      'finance-sec-press-releases',
    ]);
  });

  test('provider preset opens a prefilled market candle form', async ({ page }) => {
    await setupRoutes(page, {
      feeds: [],
      events: [],
      catalog: [
        {
          id: 'longbridge-market-candles',
          name: 'Longbridge Market Data',
          description: 'Preset for Longbridge equities market data.',
          feed_type: 'market_candle',
          provider: 'longbridge',
          tags: ['finance', 'market-data', 'equities', 'longbridge'],
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
        },
      ],
    });
    await goToDataFeeds(page);

    await expect(page.getByText(/^Provider presets$/)).toBeVisible({ timeout: 10_000 });
    await expect(page.getByText('Longbridge Market Data')).toBeVisible();
    await expect(page.getByText('Provider longbridge')).toBeVisible();

    await page.getByRole('button', { name: 'Use template' }).click();

    await expect(page.getByText('New Data Feed')).toBeVisible({ timeout: 5_000 });
    await expect(page.locator('input[placeholder="e.g. github-rara"]')).toHaveValue(
      'finance-longbridge-market-candles',
    );
    await expect(page.locator('input[placeholder="binance"]')).toHaveValue('longbridge');
  });

  test('provider preset can be configured and enabled from the catalog', async ({ page }) => {
    const state = {
      feeds: [] as DataFeedConfig[],
      events: [] as FeedEvent[],
      lastCatalogEnableBody: null as unknown,
      catalog: [
        {
          id: 'longbridge-market-candles',
          name: 'Longbridge Market Data',
          description: 'Preset for Longbridge equities market data.',
          feed_type: 'market_candle' as const,
          tags: ['finance', 'market-data', 'equities', 'longbridge'],
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
        },
      ],
    };
    await setupRoutes(page, state);
    await goToDataFeeds(page);

    await page.getByRole('button', { name: 'Configure' }).click();

    await expect(page.getByText('Configure Longbridge Market Data')).toBeVisible({
      timeout: 5_000,
    });
    await page
      .locator('input[placeholder="https://market-data.example/candles/latest"]')
      .fill('https://market-data.local/longbridge/candles/latest');
    await page.locator('input[placeholder="BTCUSDT, ETHUSDT"]').fill('AAPL.US, NVDA.US');
    await page.locator('input[placeholder="1m, 15m, 1h"]').fill('1d');
    await page.getByRole('button', { name: 'Enable source' }).click();

    await expect(
      page.getByRole('button', { name: 'finance-longbridge-market-candles' }),
    ).toBeVisible({ timeout: 5_000 });
    expect(state.lastCatalogEnableBody).toMatchObject({
      transport: {
        url: 'https://market-data.local/longbridge/candles/latest',
        venue: 'longbridge',
        symbols: ['AAPL.US', 'NVDA.US'],
        timeframes: ['1d'],
      },
    });
  });

  test('finance watch configure opens the focused data feed preset', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await setupRoutes(page, {
      feeds: [],
      events: [],
      sessions: [makeChatSession({ key: 'session-1' })],
      financeSubscriptions: { subscriptions: [], count: 0 },
      catalog: [
        {
          id: 'longbridge-market-candles',
          name: 'Longbridge Market Data',
          description: 'Preset for Longbridge equities market data.',
          feed_type: 'market_candle',
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
        },
      ],
    });

    await page.goto('/chat/session-1');

    await expect(page.getByText('Finance watches')).toBeVisible({ timeout: 10_000 });
    await expect(page.getByText('Longbridge Market Data', { exact: true })).toBeVisible();
    await page.getByRole('button', { name: 'Configure' }).click();

    await expect(page.getByText('Configure Longbridge Market Data')).toBeVisible({
      timeout: 5_000,
    });
    await expect(page.locator('input[placeholder="binance"]')).toHaveValue('longbridge');
    await expect(
      page.locator('input[placeholder="https://market-data.example/candles/latest"]'),
    ).toBeVisible();
  });

  test('finance watch bundle enables ready sources and subscribes the session', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    const fed: FeedCatalogEntry = {
      id: 'fed-press-releases',
      name: 'Federal Reserve Press Releases',
      description: 'Official Federal Reserve press releases.',
      feed_type: 'rss',
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
    const state = {
      feeds: [] as DataFeedConfig[],
      events: [] as FeedEvent[],
      sessions: [makeChatSession({ key: 'session-1' })],
      financeSubscriptions: { subscriptions: [], count: 0 },
      catalog: [fed, sec],
      financeBundles: {
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
      },
    };
    await setupRoutes(page, state);

    await page.goto('/chat/session-1');

    await expect(page.getByText('Finance watches')).toBeVisible({ timeout: 10_000 });
    await expect(page.getByText('Curated bundles')).toBeVisible();
    await expect(page.getByText('Macro News', { exact: true })).toBeVisible();
    await expect(page.getByText('0/2 sources on')).toBeVisible();

    await page.getByRole('button', { name: 'Enable bundle' }).click();

    await expect(page.getByText('2/2 sources on')).toBeVisible({ timeout: 10_000 });
    expect(state.feeds.map((feed) => feed.name).sort()).toEqual([
      'finance-fed-press-releases',
      'finance-sec-press-releases',
    ]);

    await page.getByRole('button', { name: 'Watch bundle' }).click();

    const macroBundle = page
      .getByText('Macro News', { exact: true })
      .locator('xpath=ancestor::div[contains(@class, "rounded-md")][1]');
    await expect(macroBundle.getByRole('button', { name: 'Unwatch' })).toBeVisible({
      timeout: 10_000,
    });
    expect(state.financeSubscriptions.subscriptions).toHaveLength(1);
    expect(state.financeSubscriptions.subscriptions[0].session_key).toBe('session-1');
    expect(state.financeSubscriptions.subscriptions[0].source_names.sort()).toEqual([
      'finance-fed-press-releases',
      'finance-sec-press-releases',
    ]);
  });

  // -----------------------------------------------------------------------
  // 3. Create a polling feed
  // -----------------------------------------------------------------------

  test('create a polling feed', async ({ page }) => {
    await setupRoutes(page, { feeds: [], events: [] });
    await goToDataFeeds(page);

    // Click "Create Feed" button in the empty state.
    await page.getByRole('button', { name: /Create Feed/ }).click();

    // Dialog should open.
    await expect(page.getByText('New Data Feed')).toBeVisible({ timeout: 5_000 });

    // Fill name.
    const nameInput = page.locator('input[placeholder="e.g. github-rara"]');
    await nameInput.fill('yahoo-aapl');

    // Type defaults to Polling — verify it is selected.
    await expect(page.getByText('Polling')).toBeVisible();

    // Fill URL.
    const urlInput = page.locator('input[placeholder="https://api.example.com/data"]');
    await urlInput.fill(YAHOO_API_URL);

    // Fill tags.
    const tagsInput = page.locator('input[placeholder="stock, yahoo, aapl"]');
    await tagsInput.fill('stock, yahoo, aapl');

    // Click Create.
    await page.getByRole('button', { name: 'Create' }).click();

    // Dialog should close and the feed should appear in the list.
    await expect(page.getByText('yahoo-aapl')).toBeVisible({ timeout: 5_000 });

    // Verify status badge shows Running.
    await expect(page.getByText('Running')).toBeVisible();
  });

  // -----------------------------------------------------------------------
  // 4. Feed list renders existing feeds
  // -----------------------------------------------------------------------

  test('feed list renders existing feeds with correct columns', async ({ page }) => {
    const feed = makeFeed();
    await setupRoutes(page, { feeds: [feed], events: [] });
    await goToDataFeeds(page);

    // Name column.
    await expect(page.getByText('yahoo-aapl')).toBeVisible({ timeout: 10_000 });

    // Type badge.
    await expect(page.getByText('Polling')).toBeVisible();

    // Status badge.
    await expect(page.getByText('Running')).toBeVisible();

    // Runtime summary columns.
    await expect(page.getByText('0 events')).toBeVisible();
    await expect(page.getByText('No events yet')).toBeVisible();

    // Tags.
    await expect(page.getByText('stock')).toBeVisible();
    await expect(page.getByText('yahoo', { exact: true })).toBeVisible();
    await expect(page.getByText('aapl', { exact: true })).toBeVisible();
  });

  // -----------------------------------------------------------------------
  // 5. View event history for a feed
  // -----------------------------------------------------------------------

  test('view event history for a feed', async ({ page }) => {
    const feed = makeFeed();
    const events = [makeEvent()];
    await setupRoutes(page, { feeds: [feed], events });
    await goToDataFeeds(page);

    // Click the feed name to navigate to event history.
    await page.getByText('yahoo-aapl').click();

    // Should see the Back button.
    await expect(page.getByRole('button', { name: 'Back' })).toBeVisible({ timeout: 5_000 });

    // Should see the feed info card with name and status.
    await expect(page.getByText('yahoo-aapl')).toBeVisible();
    await expect(page.getByText('Running')).toBeVisible();

    // Should see event table headers.
    await expect(page.getByRole('columnheader', { name: 'Time' })).toBeVisible();
    await expect(page.getByRole('columnheader', { name: 'Type' })).toBeVisible();
    await expect(page.getByRole('columnheader', { name: 'Size' })).toBeVisible();

    // Should see the event type badge.
    await expect(page.getByText('poll_response')).toBeVisible();
  });

  // -----------------------------------------------------------------------
  // 6. View event detail with JSON payload
  // -----------------------------------------------------------------------

  test('view event detail with JSON payload', async ({ page }) => {
    const feed = makeFeed();
    const events = [makeEvent()];
    await setupRoutes(page, { feeds: [feed], events });
    await goToDataFeeds(page);

    // Navigate to event history.
    await page.getByText('yahoo-aapl').click();
    await expect(page.getByRole('button', { name: 'Back' })).toBeVisible({ timeout: 5_000 });

    // Click the event row to open the detail sheet. Target the event-type
    // badge cell rather than the <tr>: on the 390px mobile viewport the row
    // is wider than the panel, so the row's center point sits under the
    // settings sidebar and a raw row click never lands.
    const eventRow = page.locator('tr.cursor-pointer').first();
    await eventRow.getByText('poll_response').click();

    // The Sheet should open — look for the event ID in the sheet header.
    await expect(page.getByText('evt-1')).toBeVisible({ timeout: 5_000 });

    // Payload section should show the JsonTree with Yahoo Finance keys.
    await expect(page.getByText('Payload')).toBeVisible();

    // The JsonTree renders top-level keys visible, nested ones are collapsed.
    // "chart:" and "result:" are visible at their respective nesting levels.
    await expect(page.getByText(/chart:/)).toBeVisible();
    await expect(page.getByText(/result:/)).toBeVisible();

    // "error: null" is visible at the second level of the tree.
    await expect(page.getByText(/error.*null/)).toBeVisible();

    // Copy button should be present.
    await expect(page.getByRole('button', { name: 'Copy' })).toBeVisible();
  });

  // -----------------------------------------------------------------------
  // 7. Toggle feed enabled/disabled
  // -----------------------------------------------------------------------

  test('toggle feed enabled/disabled', async ({ page }) => {
    const feed = makeFeed({ enabled: true, status: 'running' });
    await setupRoutes(page, { feeds: [feed], events: [] });
    await goToDataFeeds(page);

    // Feed should show Running initially.
    await expect(page.getByText('Running')).toBeVisible({ timeout: 10_000 });

    // Click the toggle switch.
    const toggle = page.getByRole('switch');
    await toggle.click();

    // After toggle, the status should change to Disabled.
    await expect(page.getByText('Disabled')).toBeVisible({ timeout: 5_000 });
  });

  // -----------------------------------------------------------------------
  // 8. Delete a feed
  // -----------------------------------------------------------------------

  test('delete a feed', async ({ page }) => {
    const feed = makeFeed();
    await setupRoutes(page, { feeds: [feed], events: [] });
    await goToDataFeeds(page);

    // Feed should be visible.
    await expect(page.getByText('yahoo-aapl')).toBeVisible({ timeout: 10_000 });

    // Click the delete button (Trash2 icon button with destructive styling).
    const deleteButton = page.locator('button.text-destructive');
    await deleteButton.click();

    // Confirmation dialog should appear.
    await expect(page.getByText('Delete Feed')).toBeVisible({ timeout: 5_000 });
    await expect(page.getByText('This will permanently remove this feed')).toBeVisible();

    // Click the destructive Delete button in the confirmation dialog.
    const confirmDelete = page.locator('[role="dialog"]').getByRole('button', { name: 'Delete' });
    await confirmDelete.click();

    // Feed should disappear and empty state should show.
    await expect(page.getByText('No data feeds configured')).toBeVisible({
      timeout: 5_000,
    });
  });

  // -----------------------------------------------------------------------
  // 9. Navigate back from event history to feed list
  // -----------------------------------------------------------------------

  test('navigate back from event history to feed list', async ({ page }) => {
    const feed = makeFeed();
    await setupRoutes(page, { feeds: [feed], events: [makeEvent()] });
    await goToDataFeeds(page);

    // Go to event history.
    await page.getByText('yahoo-aapl').click();
    await expect(page.getByRole('button', { name: 'Back' })).toBeVisible({ timeout: 5_000 });

    // Click Back.
    await page.getByRole('button', { name: 'Back' }).click();

    // Should return to the feed list with the "New Feed" button visible.
    await expect(page.getByRole('button', { name: 'New Feed' })).toBeVisible({ timeout: 5_000 });

    // Feed should still be in the list.
    await expect(page.getByText('yahoo-aapl')).toBeVisible();
  });

  // -----------------------------------------------------------------------
  // 10. Event history shows empty state for no events
  // -----------------------------------------------------------------------

  test('event history shows empty state when no events', async ({ page }) => {
    const feed = makeFeed();
    await setupRoutes(page, { feeds: [feed], events: [] });
    await goToDataFeeds(page);

    // Navigate to event history.
    await page.getByText('yahoo-aapl').click();
    await expect(page.getByRole('button', { name: 'Back' })).toBeVisible({ timeout: 5_000 });

    // Should show "No events in this time range".
    await expect(page.getByText('No events in this time range')).toBeVisible({ timeout: 5_000 });
  });
});
