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

import { BrowserRouter, Routes, Route, Navigate } from 'react-router';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { AuthProvider } from '@/contexts/AuthContext';
import { ServerStatusProvider } from '@/components/ServerStatusProvider';
import ProtectedRoute from '@/components/ProtectedRoute';
import DashboardLayout from '@/layouts/DashboardLayout';
import Login from '@/pages/Login';
import AgentConsole from '@/pages/AgentConsole';
import Docs from '@/pages/Docs';
import Settings from '@/pages/Settings';
import KernelTop from '@/pages/KernelTop';

const queryClient = new QueryClient();

export default function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <AuthProvider>
        <ServerStatusProvider>
          <BrowserRouter>
            <Routes>
              {/* Public routes */}
              <Route path="/login" element={<Login />} />

              {/* Protected routes */}
              <Route element={<ProtectedRoute />}>
                <Route element={<DashboardLayout />}>
                  <Route index element={<Navigate to="/agent" replace />} />
                  <Route path="agent" element={<AgentConsole />} />
                  <Route path="docs" element={<Docs />} />
                  <Route path="settings" element={<Settings />} />
                  <Route path="kernel-top" element={<KernelTop />} />

                  {/* Redirects for old routes */}
                  <Route path="chat" element={<Navigate to="/agent?tab=chat" replace />} />
                  <Route path="skills" element={<Navigate to="/settings?section=skills" replace />} />
                  <Route path="mcp" element={<Navigate to="/settings?section=mcp" replace />} />
                </Route>
              </Route>
            </Routes>
          </BrowserRouter>
        </ServerStatusProvider>
      </AuthProvider>
    </QueryClientProvider>
  );
}
