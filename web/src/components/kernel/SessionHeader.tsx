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

import { CheckCircle2, Clock, Hash, Loader2, MessageSquare, Wrench, Zap } from 'lucide-react';

import { MetadataChip } from './MetadataChip';

export interface SessionHeaderProps {
  manifestName: string;
  agentId: string;
  state: string;
  uptimeMs: number;
  llmCalls: number;
  toolCalls: number;
  tokensConsumed: number;
  isStreaming: boolean;
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

/**
 * Detail panel header: session name, state badge, and metadata chips.
 */
export function SessionHeader({
  manifestName,
  agentId,
  state,
  uptimeMs,
  llmCalls,
  toolCalls,
  tokensConsumed,
  isStreaming,
}: SessionHeaderProps) {
  const alive = state.toLowerCase() === 'active' || state.toLowerCase() === 'ready';

  return (
    <div className="space-y-2 border-b px-4 py-3">
      {/* Top row: name + state */}
      <div className="flex items-center gap-2">
        <div className="flex items-center justify-center h-6 w-6 rounded-full bg-info/10 text-info shrink-0">
          <Zap className="h-3.5 w-3.5" />
        </div>
        <span className="text-sm font-medium truncate">{manifestName}</span>
        <span className="font-mono text-[10px] text-muted-foreground/50">
          {agentId.slice(0, 8)}
        </span>

        {/* State badge */}
        {alive ? (
          <span className="inline-flex items-center gap-1 rounded-full bg-info/15 px-2 py-0.5 text-xs font-medium text-info">
            {state.toLowerCase() === 'active' ? (
              <Loader2 className="h-3 w-3 animate-spin" />
            ) : (
              <CheckCircle2 className="h-3 w-3" />
            )}
            {state}
          </span>
        ) : (
          <span className="inline-flex items-center gap-1 rounded-full bg-muted px-2 py-0.5 text-xs font-medium text-muted-foreground capitalize">
            {state.toLowerCase()}
          </span>
        )}

        {isStreaming && (
          <span className="ml-auto inline-flex items-center gap-1 rounded-full bg-emerald-500/15 px-2 py-0.5 text-[10px] font-medium text-emerald-600 dark:text-emerald-400">
            <span className="h-1.5 w-1.5 rounded-full bg-emerald-500 animate-pulse" />
            streaming
          </span>
        )}
      </div>

      {/* Metadata chips row */}
      <div className="flex flex-wrap items-center gap-1.5">
        <MetadataChip icon={<Clock className="h-3 w-3" />}>{formatUptime(uptimeMs)}</MetadataChip>
        <MetadataChip icon={<MessageSquare className="h-3 w-3" />}>{llmCalls} LLM</MetadataChip>
        <MetadataChip icon={<Wrench className="h-3 w-3" />}>{toolCalls} tools</MetadataChip>
        <MetadataChip icon={<Hash className="h-3 w-3" />}>
          {formatTokens(tokensConsumed)} tokens
        </MetadataChip>
      </div>
    </div>
  );
}
