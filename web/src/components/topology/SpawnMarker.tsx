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

import { CheckCircle2, GitBranch, Sparkles, XCircle } from 'lucide-react';

import { cn } from '@/lib/utils';

/**
 * Inline marker rendered inside the main timeline for each topology
 * transition: subagent spawn, subagent done, and tape fork. Kept
 * intentionally compact so a busy run does not overwhelm the message
 * stream.
 */
export type SpawnMarkerKind =
  | { kind: 'spawned'; childSession: string; manifestName: string }
  | { kind: 'done'; childSession: string; success: boolean }
  | { kind: 'forked'; forkedFrom: string; childTape: string; anchor: string | null };

export interface SpawnMarkerProps {
  marker: SpawnMarkerKind;
}

/** Truncate a session/tape key to its short suffix for display. */
function shortKey(key: string): string {
  // Keys are typically `agent::session-uuid` or similar; strip namespace
  // and keep the last segment to fit in a tight inline marker.
  const parts = key.split('::');
  const tail = parts[parts.length - 1] ?? key;
  return tail.length > 12 ? `${tail.slice(0, 8)}…${tail.slice(-3)}` : tail;
}

export function SpawnMarker({ marker }: SpawnMarkerProps) {
  if (marker.kind === 'spawned') {
    return (
      <div className="flex items-center gap-2 rounded-md border border-blue-500/30 bg-blue-500/5 px-3 py-1.5 text-xs">
        <Sparkles className="h-3.5 w-3.5 text-blue-500" />
        <span className="font-medium text-blue-700 dark:text-blue-300">spawned</span>
        <span className="text-foreground">{marker.manifestName}</span>
        <span className="ml-auto font-mono text-[10px] text-muted-foreground">
          {shortKey(marker.childSession)}
        </span>
      </div>
    );
  }

  if (marker.kind === 'done') {
    const Icon = marker.success ? CheckCircle2 : XCircle;
    const tone = marker.success
      ? 'border-emerald-500/30 bg-emerald-500/5 text-emerald-700 dark:text-emerald-300'
      : 'border-red-500/30 bg-red-500/5 text-red-700 dark:text-red-300';
    const iconTone = marker.success ? 'text-emerald-500' : 'text-red-500';
    return (
      <div className={cn('flex items-center gap-2 rounded-md border px-3 py-1.5 text-xs', tone)}>
        <Icon className={cn('h-3.5 w-3.5', iconTone)} />
        <span className="font-medium">{marker.success ? 'subagent done' : 'subagent failed'}</span>
        <span className="ml-auto font-mono text-[10px] text-muted-foreground">
          {shortKey(marker.childSession)}
        </span>
      </div>
    );
  }

  // forked
  return (
    <div className="flex items-center gap-2 rounded-md border border-purple-500/30 bg-purple-500/5 px-3 py-1.5 text-xs">
      <GitBranch className="h-3.5 w-3.5 text-purple-500" />
      <span className="font-medium text-purple-700 dark:text-purple-300">fork</span>
      <span className="font-mono text-[10px] text-muted-foreground">
        {shortKey(marker.forkedFrom)} → {shortKey(marker.childTape)}
      </span>
      {marker.anchor && (
        <span className="ml-auto rounded bg-purple-500/10 px-1.5 py-0.5 font-mono text-[10px] text-purple-700 dark:text-purple-300">
          {marker.anchor}
        </span>
      )}
    </div>
  );
}
