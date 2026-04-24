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
import { useEffect, useState } from 'react';
import { BrowserRouter, Routes, Route, useNavigate } from 'react-router';

import { ConnectionSetupDialog } from '@/components/ConnectionSetupDialog';
import { RequireAuth } from '@/components/RequireAuth';
import { ServerStatusProvider } from '@/components/ServerStatusProvider';
import {
  SettingsModalProvider,
  useSettingsModal,
} from '@/components/settings/SettingsModalProvider';
import type { SettingsPage } from '@/components/settings/SettingsPanel';
import DashboardLayout from '@/layouts/DashboardLayout';
import Dock from '@/pages/Dock';
import Docs from '@/pages/Docs';
import KernelTop from '@/pages/KernelTop';
import Login from '@/pages/Login';
import PiChat from '@/pages/PiChat';
import Subscriptions from '@/pages/Subscriptions';

const STORAGE_KEY = 'rara_backend_url';
const queryClient = new QueryClient();

const SETTINGS_PAGES: readonly SettingsPage[] = [
  'general',
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

              {/* Fullscreen pi-web-ui chat */}
              <Route
                index
                element={
                  <RequireAuth>
                    <PiChat />
                  </RequireAuth>
                }
              />

              {/* Deep-link redirect — see SettingsRoute */}
              <Route
                path="settings"
                element={
                  <RequireAuth>
                    <SettingsRoute />
                  </RequireAuth>
                }
              />

              {/* Admin pages with dashboard layout */}
              <Route
                element={
                  <RequireAuth>
                    <DashboardLayout />
                  </RequireAuth>
                }
              >
                <Route path="docs" element={<Docs />} />
                <Route path="kernel-top" element={<KernelTop />} />
                <Route path="dock" element={<Dock />} />
                <Route path="subscriptions" element={<Subscriptions />} />
              </Route>
            </Routes>
          </SettingsModalProvider>
        </BrowserRouter>
      </ServerStatusProvider>
    </QueryClientProvider>
  );
}
