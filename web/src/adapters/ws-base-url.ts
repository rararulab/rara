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

import { BASE_URL, getBackendUrl } from '@/api/client';

/**
 * Resolve the WebSocket base URL (scheme + host, no trailing slash) used by
 * every WS endpoint in the app.
 *
 * Resolution order mirrors REST (`resolveUrl` in `api/client.ts`):
 * 1. If the user has set a custom `rara_backend_url` in localStorage we
 *    derive WS from that host so REST and WS target the same backend.
 *    Without this, REST follows the override but WS always fell back to
 *    `window.location`, producing "WebSocket connection error" whenever
 *    the override pointed at a remote backend (issue #1622, #1921).
 * 2. Otherwise honour an explicit compile-time `BASE_URL`.
 * 3. Otherwise derive from the current page (Vite dev proxy path).
 *
 * Returns the WS-scheme prefix WITHOUT a trailing slash, e.g.
 * `ws://10.0.0.183:25555` or `wss://backend.example.com`.
 */
export function buildWsBaseUrl(): string {
  let base: string;

  const override = typeof window !== 'undefined' ? localStorage.getItem('rara_backend_url') : null;

  if (override) {
    base = getBackendUrl().replace(/^http/, 'ws');
  } else if ((BASE_URL as string).length > 0) {
    base = (BASE_URL as string).replace(/^http/, 'ws');
  } else {
    const loc = window.location;
    const proto = loc.protocol === 'https:' ? 'wss:' : 'ws:';
    base = `${proto}//${loc.host}`;
  }

  // Strip trailing slash so the joined path has exactly one separator.
  return base.replace(/\/$/, '');
}
