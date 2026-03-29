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

import { BrowserRouter, Routes, Route } from 'react-router';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { ServerStatusProvider } from '@/components/ServerStatusProvider';
import DashboardLayout from '@/layouts/DashboardLayout';
import PiChat from '@/pages/PiChat';
import Docs from '@/pages/Docs';
import Settings from '@/pages/Settings';
import KernelTop from '@/pages/KernelTop';
import Symphony from '@/pages/Symphony';
import Dock from '@/pages/Dock';

const queryClient = new QueryClient();

export default function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <ServerStatusProvider>
        <BrowserRouter>
          <Routes>
            {/* Fullscreen pi-web-ui chat */}
            <Route index element={<PiChat />} />

            {/* Admin pages with dashboard layout */}
            <Route element={<DashboardLayout />}>
              <Route path="docs" element={<Docs />} />
              <Route path="settings" element={<Settings />} />
              <Route path="kernel-top" element={<KernelTop />} />
              <Route path="symphony" element={<Symphony />} />
              <Route path="dock" element={<Dock />} />
            </Route>
          </Routes>
        </BrowserRouter>
      </ServerStatusProvider>
    </QueryClientProvider>
  );
}
