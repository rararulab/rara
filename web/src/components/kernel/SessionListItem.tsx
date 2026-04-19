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

import { Loader2 } from 'lucide-react';

import { cn } from '@/lib/utils';

export interface SessionListItemProps {
  manifestName: string;
  agentId: string;
  state: string;
  lastActivity: string | null;
  isSelected: boolean;
  onClick: () => void;
}

function formatRelativeTime(iso: string | null): string {
  if (!iso) return '';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  const diffMs = Date.now() - d.getTime();
  const diffSec = Math.floor(diffMs / 1000);
  if (diffSec < 5) return 'just now';
  if (diffSec < 60) return `${diffSec}s ago`;
  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return `${diffMin}m ago`;
  const diffHr = Math.floor(diffMin / 60);
  return `${diffHr}h ago`;
}

/** Whether this state means the session is alive (Active / Ready). */
function isAlive(state: string): boolean {
  const s = state.toLowerCase();
  return s === 'active' || s === 'ready';
}

/**
 * One row in the session list sidebar.
 *
 * Shows: manifest name, truncated agent ID, state dot + relative time.
 */
export function SessionListItem({
  manifestName,
  agentId,
  state,
  lastActivity,
  isSelected,
  onClick,
}: SessionListItemProps) {
  const alive = isAlive(state);

  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        'flex w-full items-start gap-2 border-b border-border/30 px-3 py-2.5 text-left transition-colors',
        isSelected ? 'bg-accent/40' : 'hover:bg-accent/20',
      )}
    >
      {/* State indicator */}
      <div className="mt-1.5 shrink-0">
        {alive && state.toLowerCase() === 'active' ? (
          <Loader2 className="h-3 w-3 animate-spin text-info" />
        ) : (
          <div
            className={cn(
              'h-2.5 w-2.5 rounded-full',
              alive ? 'bg-emerald-500' : 'bg-muted-foreground/30',
            )}
          />
        )}
      </div>

      {/* Content */}
      <div className="min-w-0 flex-1">
        <p className="truncate text-xs font-medium leading-snug text-foreground">{manifestName}</p>
        <div className="mt-0.5 flex items-center gap-1.5 text-[10px] text-muted-foreground/70">
          <span className="truncate font-mono">{agentId.slice(0, 8)}</span>
          <span>&middot;</span>
          <span className="capitalize">{state.toLowerCase()}</span>
          {lastActivity && (
            <>
              <span>&middot;</span>
              <span>{formatRelativeTime(lastActivity)}</span>
            </>
          )}
        </div>
      </div>
    </button>
  );
}
