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

// Playwright config for the live-backend e2e suite (chat.spec.ts,
// data-feeds.spec.ts). Kept separate from the harness suite because
// these tests hit a real rara backend via `npm run dev` and are not
// safe to run in CI without fixture coordination. Invoke with
// `npm run test:e2e:live` locally when the backend is already up.

import { defineConfig, devices } from '@playwright/test';

// The owner bearer token the live backend was booted with. CI sets
// RARA_E2E_OWNER_TOKEN to whatever it wrote into the isolated config.yaml.
// Locally, point this at the same value as `owner_token` in your
// `~/.config/rara/config.yaml` (or override via env when invoking
// `npm run test:e2e:live`).
const ownerToken = process.env.RARA_E2E_OWNER_TOKEN ?? 'ci-e2e-token-not-a-secret';

export default defineConfig({
  testDir: './e2e',
  testIgnore: /harness\/.*/,
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: 0,
  workers: 1,
  reporter: 'html',
  use: {
    baseURL: 'http://localhost:5173',
    trace: 'on-first-retry',
    // APIRequestContext uses extraHTTPHeaders for every request, satisfying
    // the backend admin auth middleware on /api/v1/* endpoints the live
    // specs hit directly via `request`.
    extraHTTPHeaders: {
      Authorization: `Bearer ${ownerToken}`,
    },
    // Page contexts — seed localStorage so the React app picks the token
    // up via getAccessToken() before any /api/v1/* fetch fires from the UI.
    storageState: {
      cookies: [],
      origins: [
        {
          origin: 'http://localhost:5173',
          localStorage: [
            { name: 'access_token', value: ownerToken },
            {
              name: 'auth_user',
              value: JSON.stringify({ user_id: 'ci-e2e', role: 'root', is_admin: true }),
            },
            { name: 'rara_backend_url', value: 'http://localhost:5173' },
            { name: 'onboarding_dismissed', value: 'true' },
          ],
        },
      ],
    },
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
  webServer: {
    command: 'npm run dev',
    url: 'http://localhost:5173',
    reuseExistingServer: true,
  },
});
