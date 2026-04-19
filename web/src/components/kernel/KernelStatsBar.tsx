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

import { Activity, Clock, Cpu, Hash, Loader2, RefreshCw } from 'lucide-react';
import { MetadataChip } from './MetadataChip';

interface SystemStats {
  active_sessions: number;
  total_spawned: number;
  total_completed: number;
  total_failed: number;
  global_semaphore_available: number;
  total_tokens_consumed: number;
  uptime_ms: number;
}

function formatUptime(ms: number): string {
  const totalSec = Math.floor(ms / 1000);
  const hours = Math.floor(totalSec / 3600);
  const minutes = Math.floor((totalSec % 3600) / 60);
  const seconds = totalSec % 60;
  if (hours > 0) return `${hours}h ${minutes}m`;
  if (minutes > 0) return `${minutes}m ${seconds}s`;
  return `${seconds}s`;
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
}

export interface KernelStatsBarProps {
  stats: SystemStats | undefined;
  isLoading: boolean;
  isFetching: boolean;
  autoRefresh: boolean;
  onAutoRefreshChange: (v: boolean) => void;
  onRefresh: () => void;
}

/**
 * Single-row chip bar replacing the 4 StatCards.
 *
 * Shows: active sessions, total spawned, tokens consumed, uptime,
 * plus auto-refresh toggle and manual refresh button.
 */
export function KernelStatsBar({
  stats,
  isLoading,
  isFetching,
  autoRefresh,
  onAutoRefreshChange,
  onRefresh,
}: KernelStatsBarProps) {
  return (
    <div className="flex flex-wrap items-center gap-2 border-b px-4 py-2">
      <span className="text-sm font-medium">Kernel</span>

      {isLoading ? (
        <Loader2 className="h-3 w-3 animate-spin text-muted-foreground" />
      ) : (
        <>
          <MetadataChip icon={<Activity className="h-3 w-3" />}>
            {stats?.active_sessions ?? 0} active
            <span className="ml-0.5 text-muted-foreground/60">
              / {stats?.global_semaphore_available ?? 0} free
            </span>
          </MetadataChip>
          <MetadataChip icon={<Cpu className="h-3 w-3" />}>
            {stats?.total_spawned ?? 0} spawned
          </MetadataChip>
          <MetadataChip icon={<Hash className="h-3 w-3" />}>
            {formatTokens(stats?.total_tokens_consumed ?? 0)} tokens
          </MetadataChip>
          <MetadataChip icon={<Clock className="h-3 w-3" />}>
            {formatUptime(stats?.uptime_ms ?? 0)}
          </MetadataChip>
        </>
      )}

      <div className="ml-auto flex items-center gap-2">
        <label className="flex cursor-pointer items-center gap-1.5 text-[11px] text-muted-foreground">
          <input
            type="checkbox"
            className="accent-primary h-3 w-3"
            checked={autoRefresh}
            onChange={(e) => onAutoRefreshChange(e.target.checked)}
          />
          Auto
        </label>
        <button
          type="button"
          onClick={onRefresh}
          disabled={isFetching}
          className="flex items-center justify-center rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground transition-colors disabled:opacity-50"
          title="Refresh"
        >
          <RefreshCw className={`h-3.5 w-3.5 ${isFetching ? 'animate-spin' : ''}`} />
        </button>
      </div>
    </div>
  );
}
