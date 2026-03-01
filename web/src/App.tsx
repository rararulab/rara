/*
 * Copyright 2025 Crrow
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

import { BrowserRouter, Routes, Route, Navigate } from 'react-router';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { ServerStatusProvider } from '@/components/ServerStatusProvider';
import DashboardLayout from '@/layouts/DashboardLayout';
import AgentConsole from '@/pages/AgentConsole';
import JobsWorkspace from '@/pages/JobsWorkspace';
import Docs from '@/pages/Docs';
import Settings from '@/pages/Settings';

const queryClient = new QueryClient();

export default function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <ServerStatusProvider>
        <BrowserRouter>
          <Routes>
            {/* Main layout */}
            <Route element={<DashboardLayout />}>
              <Route index element={<Navigate to="/agent" replace />} />
              <Route path="agent" element={<AgentConsole />} />
              <Route path="jobs" element={<JobsWorkspace />} />
              <Route path="docs" element={<Docs />} />
              <Route path="settings" element={<Settings />} />

              {/* Redirects for old routes */}
              <Route path="chat" element={<Navigate to="/agent?tab=chat" replace />} />
              <Route path="skills" element={<Navigate to="/settings?section=skills" replace />} />
              <Route path="mcp" element={<Navigate to="/settings?section=mcp" replace />} />
              <Route path="discovery" element={<Navigate to="/jobs?tab=discovery" replace />} />
              <Route path="applications" element={<Navigate to="/jobs?tab=applications" replace />} />
              <Route path="resumes" element={<Navigate to="/jobs?tab=resumes" replace />} />
              <Route path="interviews" element={<Navigate to="/jobs?tab=interviews" replace />} />
              <Route path="dashboard" element={<Navigate to="/jobs?tab=dashboard" replace />} />

            </Route>
          </Routes>
        </BrowserRouter>
      </ServerStatusProvider>
    </QueryClientProvider>
  );
}
