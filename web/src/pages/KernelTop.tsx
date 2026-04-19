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

import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { ShieldCheck, Zap } from 'lucide-react';
import { api } from '@/api/client';
import { KernelStatsBar } from '@/components/kernel/KernelStatsBar';
import { SessionList } from '@/components/kernel/SessionList';
import { SessionDetail } from '@/components/kernel/SessionDetail';
import { ApprovalsDrawer } from '@/components/kernel/ApprovalsDrawer';
import { Badge } from '@/components/ui/badge';

// ---------------------------------------------------------------------------
// Types (matching Rust backend — local to this page)
// ---------------------------------------------------------------------------

interface SystemStats {
  active_sessions: number;
  total_spawned: number;
  total_completed: number;
  total_failed: number;
  global_semaphore_available: number;
  total_tokens_consumed: number;
  uptime_ms: number;
}

interface SessionStats {
  agent_id: string;
  session_id: string;
  manifest_name: string;
  state: string;
  parent_id: string | null;
  children: string[];
  created_at: string;
  finished_at: string | null;
  uptime_ms: number;
  messages_received: number;
  llm_calls: number;
  tool_calls: number;
  tokens_consumed: number;
  last_activity: string | null;
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

const AUTO_REFRESH_INTERVAL = 5_000;

/**
 * Kernel Top — three-column session browser.
 *
 * ```
 * ┌─────────────────────────────────────────────────────────┐
 * │ KernelStatsBar                              [Approvals] │
 * ├──────────────┬──────────────────────────────────────────┤
 * │ SessionList  │ SessionDetail                            │
 * │ (sidebar)    │ (header + TimelineBar + event list)      │
 * └──────────────┴──────────────────────────────────────────┘
 * ```
 */
export default function KernelTop() {
  const [autoRefresh, setAutoRefresh] = useState(true);
  const [selectedSession, setSelectedSession] = useState<string | null>(null);
  const [approvalsOpen, setApprovalsOpen] = useState(false);

  // -- Data --
  const statsQuery = useQuery({
    queryKey: ['kernel-stats'],
    queryFn: () => api.get<SystemStats>('/api/v1/kernel/stats'),
    refetchInterval: autoRefresh ? AUTO_REFRESH_INTERVAL : false,
  });

  const sessionsQuery = useQuery({
    queryKey: ['kernel-sessions'],
    queryFn: () => api.get<SessionStats[]>('/api/v1/kernel/sessions'),
    refetchInterval: autoRefresh ? AUTO_REFRESH_INTERVAL : false,
  });

  const approvalsQuery = useQuery({
    queryKey: ['kernel-approvals'],
    queryFn: () => api.get<{ id: string }[]>('/api/v1/kernel/approvals'),
    refetchInterval: autoRefresh ? AUTO_REFRESH_INTERVAL : false,
  });

  const sessions = sessionsQuery.data ?? [];
  const approvalCount = approvalsQuery.data?.length ?? 0;
  const selectedStats = sessions.find((s) => s.agent_id === selectedSession);

  const handleRefresh = () => {
    statsQuery.refetch();
    sessionsQuery.refetch();
    approvalsQuery.refetch();
  };

  return (
    <div className="flex h-full flex-col">
      {/* ── Stats bar ─────────────────────────────────────────── */}
      <div className="flex items-center">
        <div className="flex-1">
          <KernelStatsBar
            stats={statsQuery.data}
            isLoading={statsQuery.isLoading}
            isFetching={statsQuery.isFetching || sessionsQuery.isFetching}
            autoRefresh={autoRefresh}
            onAutoRefreshChange={setAutoRefresh}
            onRefresh={handleRefresh}
          />
        </div>

        {/* Approvals button */}
        <button
          type="button"
          onClick={() => setApprovalsOpen(true)}
          className="flex items-center gap-1.5 border-b border-l px-3 py-2 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        >
          <ShieldCheck className="h-3.5 w-3.5" />
          Approvals
          {approvalCount > 0 && (
            <Badge variant="destructive" className="text-[10px] px-1.5">
              {approvalCount}
            </Badge>
          )}
        </button>
      </div>

      {/* ── Main area: sidebar + detail ───────────────────────── */}
      <div className="flex flex-1 min-h-0">
        {/* Left: session list */}
        <div className="flex w-56 shrink-0 flex-col border-r">
          <SessionList
            sessions={sessions}
            selectedId={selectedSession}
            onSelect={(id) => setSelectedSession((prev) => (prev === id ? null : id))}
            isLoading={sessionsQuery.isLoading}
          />
        </div>

        {/* Right: session detail or empty state */}
        <div className="flex flex-1 flex-col min-w-0">
          {selectedStats ? (
            <SessionDetail session={selectedStats} autoRefresh={autoRefresh} />
          ) : (
            <div className="flex flex-1 flex-col items-center justify-center gap-3 text-muted-foreground">
              <Zap className="h-10 w-10 opacity-10" />
              <p className="text-sm">Select a session to inspect</p>
            </div>
          )}
        </div>
      </div>

      {/* ── Approvals drawer ──────────────────────────────────── */}
      <ApprovalsDrawer open={approvalsOpen} onOpenChange={setApprovalsOpen} />
    </div>
  );
}
