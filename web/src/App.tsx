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

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { lazy, Suspense, useEffect, useState } from 'react';
import { BrowserRouter, Navigate, Routes, Route, useNavigate, useParams } from 'react-router';

import { ConnectionSetupDialog } from '@/components/ConnectionSetupDialog';
import { RequireAuth } from '@/components/RequireAuth';
import { ServerStatusProvider } from '@/components/ServerStatusProvider';
import {
  SettingsModalProvider,
  useSettingsModal,
} from '@/components/settings/SettingsModalProvider';
import type { SettingsPage } from '@/components/settings/SettingsPanel';
import DashboardLayout from '@/layouts/DashboardLayout';
import Docs from '@/pages/Docs';
import Login from '@/pages/Login';

// Chat owns the heavy vendor surface (`src/vendor/craft-ui` + tiptap,
// react-pdf, mermaid, @uiw/react-json-view). Lazy-loading it keeps
// `/login` and `/docs` off that ~4.7 MB chunk on first paint —
// see issue #2033.
const Chat = lazy(() => import('@/pages/Chat'));

/**
 * Backwards-compat redirect for the old `/topology/:rootSessionKey`
 * deep-links from `#1999`. Preserves the param so bookmarks survive
 * the rename in `#2041`.
 */
function TopologySessionRedirect() {
  const { rootSessionKey } = useParams<{ rootSessionKey?: string }>();
  return (
    <Navigate
      to={rootSessionKey ? `/chat/${encodeURIComponent(rootSessionKey)}` : '/chat'}
      replace
    />
  );
}

function RouteFallback() {
  return (
    <div className="flex h-full w-full items-center justify-center p-8 text-sm text-muted-foreground">
      Loading…
    </div>
  );
}

const STORAGE_KEY = 'rara_backend_url';
const queryClient = new QueryClient();

const SETTINGS_PAGES: readonly SettingsPage[] = [
  'appearance',
  'connection',
  'providers',
  'agents',
  'skills',
  'mcp',
  'channels',
  'tools',
  'security',
  'data-feeds',
];

function isSettingsPage(value: string | null): value is SettingsPage {
  return !!value && (SETTINGS_PAGES as readonly string[]).includes(value);
}

/**
 * Backwards-compat redirect for deep-links like `/settings?section=providers`.
 * Settings now lives in a floating modal; the old route opens the modal and
 * hops back to the root so bookmarks and external links keep working.
 */
function SettingsRoute() {
  const { openSettings } = useSettingsModal();
  const navigate = useNavigate();

  useEffect(() => {
    const raw = new URLSearchParams(window.location.search).get('section');
    openSettings(isSettingsPage(raw) ? raw : undefined);
    void navigate('/', { replace: true });
  }, [openSettings, navigate]);

  return null;
}

export default function App() {
  const [needsSetup, setNeedsSetup] = useState(() => !localStorage.getItem(STORAGE_KEY));

  return (
    <QueryClientProvider client={queryClient}>
      <ServerStatusProvider>
        {needsSetup && (
          <ConnectionSetupDialog open={needsSetup} onConnect={() => setNeedsSetup(false)} />
        )}
        <BrowserRouter>
          <SettingsModalProvider>
            <Routes>
              {/* Owner-token login — public route, must not be guarded. */}
              <Route path="login" element={<Login />} />

              {/* Deep-link redirect — see SettingsRoute */}
              <Route
                path="settings"
                element={
                  <RequireAuth>
                    <SettingsRoute />
                  </RequireAuth>
                }
              />

              {/* Admin pages with dashboard layout. The chat view is the
                  default landing page after login — see issue #1999. */}
              <Route
                element={
                  <RequireAuth>
                    <DashboardLayout />
                  </RequireAuth>
                }
              >
                <Route
                  index
                  element={
                    <Suspense fallback={<RouteFallback />}>
                      <Chat />
                    </Suspense>
                  }
                />
                <Route path="docs" element={<Docs />} />
                <Route
                  path="chat"
                  element={
                    <Suspense fallback={<RouteFallback />}>
                      <Chat />
                    </Suspense>
                  }
                />
                <Route
                  path="chat/:rootSessionKey"
                  element={
                    <Suspense fallback={<RouteFallback />}>
                      <Chat />
                    </Suspense>
                  }
                />
                {/* `/topology[/:rootSessionKey]` → `/chat[/:rootSessionKey]`
                    redirects keep `#1999` deep-links alive after the
                    `#2041` rename. */}
                <Route path="topology" element={<Navigate to="/chat" replace />} />
                <Route path="topology/:rootSessionKey" element={<TopologySessionRedirect />} />
              </Route>
            </Routes>
          </SettingsModalProvider>
        </BrowserRouter>
      </ServerStatusProvider>
    </QueryClientProvider>
  );
}
