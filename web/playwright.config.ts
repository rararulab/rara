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

import { defineConfig, devices } from '@playwright/test';

// The harness/ suite is designed to run headless against a `vite
// preview` build with every network request intercepted, so it
// renders without a live rara backend. The existing backend-driven
// e2e specs keep their own `webServer: npm run dev` contract — we
// run them opt-in via `npm run test:e2e:live`.
const isCi = !!process.env.CI;

export default defineConfig({
  testDir: './e2e',
  testMatch: /harness\/.*\.spec\.ts$/,
  fullyParallel: true,
  forbidOnly: isCi,
  retries: isCi ? 1 : 0,
  workers: isCi ? 2 : 1,
  reporter: isCi ? [['html', { open: 'never' }], ['list']] : 'html',
  // Pin snapshot paths to omit the OS suffix — baselines are generated
  // inside the linux playwright container so CI renders identically.
  snapshotPathTemplate: '{testDir}/__screenshots__/{testFilePath}/{arg}{ext}',
  expect: {
    toHaveScreenshot: {
      // Allow minor antialiasing drift between local + CI runs.
      maxDiffPixelRatio: 0.02,
    },
  },
  use: {
    baseURL: 'http://localhost:4173',
    trace: 'on-first-retry',
  },
  projects: [
    {
      name: 'mobile',
      use: {
        ...devices['Mobile Safari'],
        viewport: { width: 390, height: 844 },
      },
    },
    {
      name: 'desktop-1280',
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 1280, height: 800 },
      },
    },
    {
      name: 'desktop-1920',
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 1920, height: 1080 },
      },
    },
  ],
  webServer: {
    // Serve the pre-built dist with a dependency-free static server so the
    // harness suite doesn't need Vite's Rollup toolchain at test time
    // (matters for Docker-based baseline generation on a different arch).
    // `serve -s` rewrites unknown paths to index.html for SPA routing.
    command: 'npx --yes serve dist -l 4173 -s',
    url: 'http://localhost:4173',
    reuseExistingServer: !isCi,
    timeout: 120_000,
  },
});
