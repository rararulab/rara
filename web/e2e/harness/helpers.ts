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

import type { Page } from '@playwright/test';

interface StubSession {
  key: string;
  title: string;
  preview?: string;
  updated_at?: string;
  created_at?: string;
  message_count?: number;
}

/** Pre-seed the backend-URL localStorage entry so the setup dialog doesn't block the page. */
export async function primeBackendUrl(page: Page, url = 'http://localhost:25555'): Promise<void> {
  await page.addInitScript((backendUrl) => {
    window.localStorage.setItem('rara_backend_url', backendUrl);
  }, url);
}

/**
 * Pre-seed the auth localStorage entries so `<RequireAuth>` doesn't redirect
 * the harness to `/login`. Keys mirror the constants in `src/api/client.ts`
 * (`ACCESS_TOKEN_KEY`, `AUTH_USER_KEY`).
 */
export async function primeAuth(page: Page): Promise<void> {
  await page.addInitScript(() => {
    window.localStorage.setItem('access_token', 'harness-token');
    window.localStorage.setItem(
      'auth_user',
      JSON.stringify({ user_id: 'harness', role: 'owner', is_admin: true }),
    );
  });
}

/**
 * Pinned "now" the harness renders against. Fixture `updated_at`
 * values in `stubApi` are anchored to this moment, and
 * `freezePageClock` installs the same instant into the page's JS
 * clock. Together they keep `formatRelativeDate` output byte-stable
 * across days so screenshot baselines don't drift against wall clock.
 */
export const HARNESS_NOW_ISO = '2025-06-15T12:00:00Z';

/**
 * Freeze the page's `Date.now()` / `new Date()` to `HARNESS_NOW_ISO`.
 * Must be called before `page.goto(...)` so the app's first render
 * sees the pinned clock.
 */
export async function freezePageClock(
  page: Page,
  isoTime: string = HARNESS_NOW_ISO,
): Promise<void> {
  await page.clock.install({ time: new Date(isoTime) });
}

/**
 * Intercept every /api/** request and serve a minimal deterministic
 * payload so the harness renders without a real backend. Only the
 * routes the tests care about need concrete data; everything else
 * returns an empty object or list so the UI degrades gracefully.
 */
export async function stubApi(
  page: Page,
  options: { sessions?: StubSession[]; settings?: Record<string, string> } = {},
): Promise<void> {
  // Default fixture dates align with the pinned page clock
  // (`HARNESS_NOW_ISO`) so `formatRelativeDate` resolves to a stable
  // string ("just now") instead of drifting with wall clock.
  const sessions = (options.sessions ?? []).map((s) => ({
    key: s.key,
    title: s.title,
    preview: s.preview ?? '',
    updated_at: s.updated_at ?? HARNESS_NOW_ISO,
    created_at: s.created_at ?? HARNESS_NOW_ISO,
    message_count: s.message_count ?? 0,
    model: null,
    model_provider: null,
    thinking_level: null,
    system_prompt: null,
    metadata: null,
  }));
  const settings = options.settings ?? {};

  await page.route('**/api/**', async (route) => {
    const url = new URL(route.request().url());
    const path = url.pathname;

    if (path.startsWith('/api/v1/health')) {
      return route.fulfill({ status: 200, body: 'ok' });
    }
    if (path.startsWith('/api/v1/chat/sessions') && path !== '/api/v1/chat/sessions') {
      // GET /api/v1/chat/sessions/{key}/messages etc.
      if (path.endsWith('/messages')) {
        return route.fulfill({ status: 200, contentType: 'application/json', body: '[]' });
      }
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({}),
      });
    }
    if (path === '/api/v1/chat/sessions') {
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(sessions),
      });
    }
    if (path.startsWith('/api/v1/settings')) {
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(settings),
      });
    }
    if (path.startsWith('/api/v1/providers')) {
      return route.fulfill({ status: 200, contentType: 'application/json', body: '[]' });
    }
    // Catch-all — keep the UI alive by returning an empty JSON object.
    return route.fulfill({ status: 200, contentType: 'application/json', body: '{}' });
  });
}
