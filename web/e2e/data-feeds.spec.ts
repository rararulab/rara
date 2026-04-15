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

import { test, expect } from "@playwright/test";

// ---------------------------------------------------------------------------
// Types — mirrors web/src/api/data-feeds.ts
// ---------------------------------------------------------------------------

interface DataFeedConfig {
  id: string;
  name: string;
  feed_type: "webhook" | "websocket" | "polling";
  tags: string[];
  transport: Record<string, unknown>;
  auth: { type: string; [key: string]: unknown } | null;
  enabled: boolean;
  status: "idle" | "running" | "error";
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

// ---------------------------------------------------------------------------
// Shared mock data
// ---------------------------------------------------------------------------

/** Yahoo Finance payload — fetched once in beforeAll, or falls back to static fixture. */
let yahooPayload: unknown;

const YAHOO_API_URL =
  "https://query1.finance.yahoo.com/v8/finance/chart/AAPL?interval=1d&range=1d";

/** Static fallback when Yahoo Finance API is unreachable. */
const YAHOO_FALLBACK = {
  chart: {
    result: [
      {
        meta: {
          currency: "USD",
          symbol: "AAPL",
          exchangeName: "NMS",
          fullExchangeName: "NasdaqGS",
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
  "llm.default_provider": "openrouter",
  "llm.providers.openrouter.enabled": "true",
  "llm.providers.openrouter.api_key": "sk-fake-key-for-e2e",
};

function makeFeed(overrides: Partial<DataFeedConfig> = {}): DataFeedConfig {
  const now = new Date().toISOString();
  return {
    id: "feed-1",
    name: "yahoo-aapl",
    feed_type: "polling",
    tags: ["stock", "yahoo", "aapl"],
    transport: {
      url: YAHOO_API_URL,
      interval_secs: 60,
      headers: {},
      method: "GET",
    },
    auth: null,
    enabled: true,
    status: "running",
    last_error: null,
    created_at: now,
    updated_at: now,
    ...overrides,
  };
}

function makeEvent(overrides: Partial<FeedEvent> = {}): FeedEvent {
  return {
    id: "evt-1",
    source_name: "yahoo-aapl",
    event_type: "poll_response",
    tags: ["stock", "yahoo", "aapl"],
    payload: yahooPayload,
    received_at: new Date().toISOString(),
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// Setup — fetch real Yahoo Finance data once for realistic payloads
// ---------------------------------------------------------------------------

test.beforeAll(async () => {
  try {
    const res = await fetch(YAHOO_API_URL, {
      headers: { "User-Agent": "Mozilla/5.0" },
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
  page: import("@playwright/test").Page,
  state: {
    feeds: DataFeedConfig[];
    events: FeedEvent[];
  },
) {
  // Suppress onboarding & connection dialogs via localStorage.
  // Using the Vite dev server URL keeps resolveUrl() pointing at same origin.
  await page.addInitScript(() => {
    // Use empty string so resolveUrl returns relative paths that page.route can intercept.
    // hasCustomBackendUrl() needs a truthy value to suppress ConnectionSetupDialog.
    localStorage.setItem("rara_backend_url", "http://localhost:5173");
    localStorage.setItem("onboarding_dismissed", "true");
  });

  // Health check.
  await page.route("**/api/v1/health", (route) =>
    route.fulfill({ status: 200, json: { status: "ok" } }),
  );

  // Settings — return a configured provider so onboarding is suppressed.
  await page.route("**/api/v1/settings", (route) =>
    route.fulfill({ status: 200, json: MOCK_SETTINGS }),
  );

  // Data feeds list + create.
  await page.route("**/api/v1/data-feeds", async (route, request) => {
    const method = request.method();
    if (method === "GET") {
      await route.fulfill({ json: state.feeds });
    } else if (method === "POST") {
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
        status: "running",
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
  await page.route("**/api/v1/data-feeds/*/toggle", async (route, request) => {
    if (request.method() === "PUT") {
      const url = request.url();
      const idMatch = url.match(/data-feeds\/([^/]+)\/toggle/);
      const id = idMatch?.[1];
      const feed = state.feeds.find((f) => f.id === id);
      if (feed) {
        feed.enabled = !feed.enabled;
        feed.status = feed.enabled ? "running" : "idle";
        await route.fulfill({ json: feed });
      } else {
        await route.fulfill({ status: 404, json: { error: "not found" } });
      }
    } else {
      await route.continue();
    }
  });

  // Feed events — must be registered before the single-feed catch-all.
  await page.route("**/api/v1/data-feeds/*/events*", async (route) => {
    await route.fulfill({
      json: {
        events: state.events,
        total: state.events.length,
        has_more: false,
      },
    });
  });

  // Single feed operations (GET/PUT/DELETE by id).
  await page.route(
    /\/api\/v1\/data-feeds\/[^/]+$/,
    async (route, request) => {
      const method = request.method();
      const url = request.url();
      const idMatch = url.match(/data-feeds\/([^/]+)$/);
      const id = idMatch?.[1];

      if (method === "DELETE") {
        state.feeds = state.feeds.filter((f) => f.id !== id);
        await route.fulfill({ status: 204 });
      } else if (method === "PUT") {
        const feed = state.feeds.find((f) => f.id === id);
        if (feed) {
          const body = request.postDataJSON();
          Object.assign(feed, body, { updated_at: new Date().toISOString() });
          await route.fulfill({ json: feed });
        } else {
          await route.fulfill({ status: 404, json: { error: "not found" } });
        }
      } else if (method === "GET") {
        const feed = state.feeds.find((f) => f.id === id);
        if (feed) {
          await route.fulfill({ json: feed });
        } else {
          await route.fulfill({ status: 404, json: { error: "not found" } });
        }
      } else {
        await route.continue();
      }
    },
  );
}

/** Navigate directly to the Data Feeds settings tab. */
async function goToDataFeeds(page: import("@playwright/test").Page) {
  await page.goto("/settings?section=data-feeds");
  await page.waitForLoadState("networkidle");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

test.describe("Data Feeds Management", () => {

  // -----------------------------------------------------------------------
  // 1. Navigate to Data Feeds tab
  // -----------------------------------------------------------------------

  test("navigate to Data Feeds tab in settings", async ({ page }) => {
    await setupRoutes(page, { feeds: [], events: [] });
    await page.goto("/settings");

    // Click the Data Feeds sidebar button.
    const navButton = page.locator("button", { hasText: "Data Feeds" });
    await expect(navButton).toBeVisible({ timeout: 10_000 });
    await navButton.click();

    // The Data Feeds panel should render.
    await expect(
      page.getByText("External data sources that push events into rara."),
    ).toBeVisible();
  });

  // -----------------------------------------------------------------------
  // 2. Empty state
  // -----------------------------------------------------------------------

  test("shows empty state when no feeds configured", async ({ page }) => {
    await setupRoutes(page, { feeds: [], events: [] });
    await goToDataFeeds(page);

    await expect(
      page.getByText("No data feeds configured"),
    ).toBeVisible({ timeout: 10_000 });
    await expect(page.getByText("Create Feed")).toBeVisible();
  });

  // -----------------------------------------------------------------------
  // 3. Create a polling feed
  // -----------------------------------------------------------------------

  test("create a polling feed", async ({ page }) => {
    await setupRoutes(page, { feeds: [], events: [] });
    await goToDataFeeds(page);

    // Click "Create Feed" button in the empty state.
    await page.getByRole("button", { name: /Create Feed/ }).click();

    // Dialog should open.
    await expect(page.getByText("New Data Feed")).toBeVisible({ timeout: 5_000 });

    // Fill name.
    const nameInput = page.locator('input[placeholder="e.g. github-rara"]');
    await nameInput.fill("yahoo-aapl");

    // Type defaults to Polling — verify it is selected.
    await expect(page.getByText("Polling")).toBeVisible();

    // Fill URL.
    const urlInput = page.locator(
      'input[placeholder="https://api.example.com/data"]',
    );
    await urlInput.fill(YAHOO_API_URL);

    // Fill tags.
    const tagsInput = page.locator('input[placeholder="stock, yahoo, aapl"]');
    await tagsInput.fill("stock, yahoo, aapl");

    // Click Create.
    await page.getByRole("button", { name: "Create" }).click();

    // Dialog should close and the feed should appear in the list.
    await expect(page.getByText("yahoo-aapl")).toBeVisible({ timeout: 5_000 });

    // Verify status badge shows Running.
    await expect(page.getByText("Running")).toBeVisible();
  });

  // -----------------------------------------------------------------------
  // 4. Feed list renders existing feeds
  // -----------------------------------------------------------------------

  test("feed list renders existing feeds with correct columns", async ({
    page,
  }) => {
    const feed = makeFeed();
    await setupRoutes(page, { feeds: [feed], events: [] });
    await goToDataFeeds(page);

    // Name column.
    await expect(page.getByText("yahoo-aapl")).toBeVisible({ timeout: 10_000 });

    // Type badge.
    await expect(page.getByText("Polling")).toBeVisible();

    // Status badge.
    await expect(page.getByText("Running")).toBeVisible();

    // Tags.
    await expect(page.getByText("stock")).toBeVisible();
    await expect(page.getByText("yahoo", { exact: true })).toBeVisible();
    await expect(page.getByText("aapl", { exact: true })).toBeVisible();
  });

  // -----------------------------------------------------------------------
  // 5. View event history for a feed
  // -----------------------------------------------------------------------

  test("view event history for a feed", async ({ page }) => {
    const feed = makeFeed();
    const events = [makeEvent()];
    await setupRoutes(page, { feeds: [feed], events });
    await goToDataFeeds(page);

    // Click the feed name to navigate to event history.
    await page.getByText("yahoo-aapl").click();

    // Should see the Back button.
    await expect(
      page.getByRole("button", { name: "Back" }),
    ).toBeVisible({ timeout: 5_000 });

    // Should see the feed info card with name and status.
    await expect(page.getByText("yahoo-aapl")).toBeVisible();
    await expect(page.getByText("Running")).toBeVisible();

    // Should see event table headers.
    await expect(page.getByRole("columnheader", { name: "Time" })).toBeVisible();
    await expect(page.getByRole("columnheader", { name: "Type" })).toBeVisible();
    await expect(page.getByRole("columnheader", { name: "Size" })).toBeVisible();

    // Should see the event type badge.
    await expect(page.getByText("poll_response")).toBeVisible();
  });

  // -----------------------------------------------------------------------
  // 6. View event detail with JSON payload
  // -----------------------------------------------------------------------

  test("view event detail with JSON payload", async ({ page }) => {
    const feed = makeFeed();
    const events = [makeEvent()];
    await setupRoutes(page, { feeds: [feed], events });
    await goToDataFeeds(page);

    // Navigate to event history.
    await page.getByText("yahoo-aapl").click();
    await expect(
      page.getByRole("button", { name: "Back" }),
    ).toBeVisible({ timeout: 5_000 });

    // Click the event row to open the detail sheet.
    const eventRow = page.locator("tr.cursor-pointer").first();
    await eventRow.click();

    // The Sheet should open — look for the event ID in the sheet header.
    await expect(page.getByText("evt-1")).toBeVisible({ timeout: 5_000 });

    // Payload section should show the JsonTree with Yahoo Finance keys.
    await expect(page.getByText("Payload")).toBeVisible();

    // The JsonTree renders top-level keys visible, nested ones are collapsed.
    // "chart:" and "result:" are visible at their respective nesting levels.
    await expect(page.getByText(/chart:/)).toBeVisible();
    await expect(page.getByText(/result:/)).toBeVisible();

    // "error: null" is visible at the second level of the tree.
    await expect(page.getByText(/error.*null/)).toBeVisible();

    // Copy button should be present.
    await expect(page.getByRole("button", { name: "Copy" })).toBeVisible();
  });

  // -----------------------------------------------------------------------
  // 7. Toggle feed enabled/disabled
  // -----------------------------------------------------------------------

  test("toggle feed enabled/disabled", async ({ page }) => {
    const feed = makeFeed({ enabled: true, status: "running" });
    await setupRoutes(page, { feeds: [feed], events: [] });
    await goToDataFeeds(page);

    // Feed should show Running initially.
    await expect(page.getByText("Running")).toBeVisible({ timeout: 10_000 });

    // Click the toggle switch.
    const toggle = page.getByRole("switch");
    await toggle.click();

    // After toggle, the status should change to Disabled.
    await expect(page.getByText("Disabled")).toBeVisible({ timeout: 5_000 });
  });

  // -----------------------------------------------------------------------
  // 8. Delete a feed
  // -----------------------------------------------------------------------

  test("delete a feed", async ({ page }) => {
    const feed = makeFeed();
    await setupRoutes(page, { feeds: [feed], events: [] });
    await goToDataFeeds(page);

    // Feed should be visible.
    await expect(page.getByText("yahoo-aapl")).toBeVisible({ timeout: 10_000 });

    // Click the delete button (Trash2 icon button with destructive styling).
    const deleteButton = page.locator("button.text-destructive");
    await deleteButton.click();

    // Confirmation dialog should appear.
    await expect(page.getByText("Delete Feed")).toBeVisible({ timeout: 5_000 });
    await expect(
      page.getByText("This will permanently remove this feed"),
    ).toBeVisible();

    // Click the destructive Delete button in the confirmation dialog.
    const confirmDelete = page
      .locator('[role="dialog"]')
      .getByRole("button", { name: "Delete" });
    await confirmDelete.click();

    // Feed should disappear and empty state should show.
    await expect(page.getByText("No data feeds configured")).toBeVisible({
      timeout: 5_000,
    });
  });

  // -----------------------------------------------------------------------
  // 9. Navigate back from event history to feed list
  // -----------------------------------------------------------------------

  test("navigate back from event history to feed list", async ({ page }) => {
    const feed = makeFeed();
    await setupRoutes(page, { feeds: [feed], events: [makeEvent()] });
    await goToDataFeeds(page);

    // Go to event history.
    await page.getByText("yahoo-aapl").click();
    await expect(
      page.getByRole("button", { name: "Back" }),
    ).toBeVisible({ timeout: 5_000 });

    // Click Back.
    await page.getByRole("button", { name: "Back" }).click();

    // Should return to the feed list with the "New Feed" button visible.
    await expect(
      page.getByRole("button", { name: "New Feed" }),
    ).toBeVisible({ timeout: 5_000 });

    // Feed should still be in the list.
    await expect(page.getByText("yahoo-aapl")).toBeVisible();
  });

  // -----------------------------------------------------------------------
  // 10. Event history shows empty state for no events
  // -----------------------------------------------------------------------

  test("event history shows empty state when no events", async ({ page }) => {
    const feed = makeFeed();
    await setupRoutes(page, { feeds: [feed], events: [] });
    await goToDataFeeds(page);

    // Navigate to event history.
    await page.getByText("yahoo-aapl").click();
    await expect(
      page.getByRole("button", { name: "Back" }),
    ).toBeVisible({ timeout: 5_000 });

    // Should show "No events in this time range".
    await expect(
      page.getByText("No events in this time range"),
    ).toBeVisible({ timeout: 5_000 });
  });
});
