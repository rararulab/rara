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

import { useQuery } from '@tanstack/react-query';
import { useMemo, useState } from 'react';
import { Outlet, useLocation } from 'react-router';

import { settingsApi } from '@/api/client';
import OnboardingModal, { isOnboardingDismissed } from '@/components/OnboardingModal';
import NavRail from '@/components/shell/NavRail';
import {
  PageStatusProvider,
  usePageStatus,
  type PageLiveStatus,
} from '@/components/shell/PageStatusContext';
import ThemeToggle from '@/components/ThemeToggle';
import { cn } from '@/lib/utils';

/** Routes that need zero padding in the main content area. */
const FULL_BLEED_ROUTES = new Set(['/agent', '/docs']);

/** Routes that need full bleed when they match as a prefix. */
const FULL_BLEED_PREFIXES: string[] = [];

const SETTINGS_KEYS = {
  defaultProvider: 'llm.default_provider',
  openrouterEnabled: 'llm.providers.openrouter.enabled',
  openrouterApiKey: 'llm.providers.openrouter.api_key',
  ollamaEnabled: 'llm.providers.ollama.enabled',
  ollamaApiKey: 'llm.providers.ollama.api_key',
  ollamaBaseUrl: 'llm.providers.ollama.base_url',
  codexEnabled: 'llm.providers.codex.enabled',
} as const;

/**
 * Per-route metadata consumed by the slim top bar. Iterated in
 * declaration order; the first matching predicate wins, so list more
 * specific patterns before broader ones.
 *
 * Kept as a lookup table rather than `<Route handle>` + `useMatches()`
 * because the app still mounts a plain `BrowserRouter` from
 * `react-router` (see `App.tsx`). `useMatches()` is a data-router
 * primitive — calling it under `BrowserRouter` throws — and a small
 * pathname → handle table is smaller than migrating the whole router
 * topology to `createBrowserRouter` for a top-bar title.
 */
interface RouteHandle {
  title: string;
  showLiveIndicator?: boolean;
}

const ROUTE_HANDLES: ReadonlyArray<{ test: (path: string) => boolean; handle: RouteHandle }> = [
  // First match wins; declaration order matters. The list MUST cover
  // every layout-mounted route in `App.tsx` — there is no fallback
  // entry, so a missed route renders an empty top-bar title. Keep this
  // list aligned with the `<Route>` children of `<DashboardLayout />`.
  //
  // `/chat` covers `/chat`, `/chat/:rootSessionKey`, and the index
  // route (`/`) which also renders `<Chat />`.
  {
    test: (p) => p === '/' || p === '/chat' || p.startsWith('/chat/'),
    handle: { title: 'Chat', showLiveIndicator: true },
  },
  { test: (p) => p === '/docs' || p.startsWith('/docs/'), handle: { title: 'Documentation' } },
  // `/topology` and `/topology/:key` are pure redirects to `/chat` (see
  // `App.tsx`); they never render under this layout, so they
  // deliberately have no entry here.
];

function resolveHandle(pathname: string): RouteHandle | null {
  for (const entry of ROUTE_HANDLES) {
    if (entry.test(pathname)) return entry.handle;
  }
  return null;
}

function hasConfiguredLlmProvider(settings: Record<string, string> | undefined): boolean {
  if (!settings) {
    return false;
  }

  const defaultProvider = settings[SETTINGS_KEYS.defaultProvider]?.trim();
  if (!defaultProvider) {
    return false;
  }

  switch (defaultProvider) {
    case 'openrouter':
      return (
        settings[SETTINGS_KEYS.openrouterEnabled] === 'true' &&
        Boolean(settings[SETTINGS_KEYS.openrouterApiKey]?.trim())
      );
    case 'ollama':
      return (
        settings[SETTINGS_KEYS.ollamaEnabled] === 'true' &&
        Boolean(settings[SETTINGS_KEYS.ollamaApiKey]?.trim()) &&
        Boolean(settings[SETTINGS_KEYS.ollamaBaseUrl]?.trim())
      );
    case 'codex':
      return settings[SETTINGS_KEYS.codexEnabled] === 'true';
    default:
      return false;
  }
}

/**
 * Tiny live-state pill rendered next to the page title in the top bar
 * when the current route opts in via `handle.showLiveIndicator`. Reads
 * the page-published status (see `PageStatusContext`) — never opens its
 * own subscription, because the page already owns one.
 */
function LiveIndicator({ status }: { status: PageLiveStatus | null }) {
  if (!status) return null;

  const variants: Record<PageLiveStatus, { dot: string; text: string; label: string }> = {
    idle: { dot: 'bg-muted-foreground/50', text: 'text-muted-foreground', label: 'idle' },
    connecting: {
      dot: 'bg-amber-500 animate-pulse',
      text: 'text-amber-600 dark:text-amber-400',
      label: 'connecting',
    },
    live: {
      dot: 'bg-emerald-500',
      text: 'text-emerald-600 dark:text-emerald-400',
      label: 'live',
    },
    reconnecting: {
      dot: 'bg-amber-500 animate-pulse',
      text: 'text-amber-600 dark:text-amber-400',
      label: 'reconnecting',
    },
    closed: { dot: 'bg-red-500', text: 'text-red-600 dark:text-red-400', label: 'closed' },
  };
  const v = variants[status];
  return (
    <span className={cn('inline-flex items-center gap-1.5 text-[11px]', v.text)}>
      <span className={cn('h-1.5 w-1.5 rounded-full', v.dot)} />
      {v.label}
    </span>
  );
}

/** Slim top bar — page title (left) + theme toggle (right). */
function TopBar() {
  const { pathname } = useLocation();
  const status = usePageStatus();
  const handle = useMemo(() => resolveHandle(pathname), [pathname]);

  return (
    <div className="flex h-10 shrink-0 items-center justify-between gap-2 border-b border-border/40 bg-background/30 px-4 backdrop-blur-sm">
      <div className="flex min-w-0 items-center gap-2">
        {handle?.title && (
          <h1 className="truncate text-sm font-medium text-foreground [text-wrap:balance]">
            {handle.title}
          </h1>
        )}
        {handle?.showLiveIndicator && <LiveIndicator status={status} />}
      </div>
      <div className="flex items-center">
        <ThemeToggle />
      </div>
    </div>
  );
}

export default function DashboardLayout() {
  const location = useLocation();
  const isFullBleed =
    FULL_BLEED_ROUTES.has(location.pathname) ||
    FULL_BLEED_PREFIXES.some((p) => location.pathname.startsWith(p));

  const settingsQuery = useQuery({
    queryKey: ['settings'],
    queryFn: () => settingsApi.list(),
  });

  // Skip onboarding entirely when dismissed or when providers are already configured.
  const providerConfigured = hasConfiguredLlmProvider(settingsQuery.data);
  const shouldShowOnboarding = !isOnboardingDismissed() && !providerConfigured;
  const [onboardingOpen, setOnboardingOpen] = useState(true);

  const handleOnboardingDismiss = () => {
    setOnboardingOpen(false);
  };

  return (
    <PageStatusProvider>
      <div className="rara-admin flex h-screen bg-transparent">
        {shouldShowOnboarding && (
          <OnboardingModal
            open={onboardingOpen}
            onDismiss={handleOnboardingDismiss}
            showLlmProviderPrompt={!hasConfiguredLlmProvider(settingsQuery.data)}
          />
        )}

        <NavRail />

        <main
          className={cn(
            'relative flex min-w-0 flex-1 flex-col',
            isFullBleed ? 'overflow-hidden' : 'overflow-auto',
          )}
        >
          <TopBar />

          <div className={cn('flex-1 min-h-0', isFullBleed ? 'p-2 md:p-3' : 'p-4 md:p-6')}>
            <Outlet />
          </div>
        </main>
      </div>
    </PageStatusProvider>
  );
}
