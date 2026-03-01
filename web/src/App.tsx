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
import { AuthProvider } from '@/contexts/AuthContext';
import { ServerStatusProvider } from '@/components/ServerStatusProvider';
import ProtectedRoute from '@/components/ProtectedRoute';
import AdminRoute from '@/components/AdminRoute';
import DashboardLayout from '@/layouts/DashboardLayout';
import Login from '@/pages/Login';
import Register from '@/pages/Register';
import AgentConsole from '@/pages/AgentConsole';
import JobsWorkspace from '@/pages/JobsWorkspace';
import Docs from '@/pages/Docs';
import Settings from '@/pages/Settings';

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
              <Route path="/register" element={<Register />} />

              {/* Protected routes */}
              <Route element={<ProtectedRoute />}>
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

                  {/* Admin routes */}
                  <Route element={<AdminRoute />}>
                    <Route path="admin/users" element={<div className="p-6 text-muted-foreground">User management coming soon.</div>} />
                  </Route>
                </Route>
              </Route>
            </Routes>
          </BrowserRouter>
        </ServerStatusProvider>
      </AuthProvider>
    </QueryClientProvider>
  );
}
