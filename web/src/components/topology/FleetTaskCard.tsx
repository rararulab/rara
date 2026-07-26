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

import { ExternalLink } from 'lucide-react';

import { formatRelativeTime } from './SessionPicker';

import { TERMINAL_STATUSES } from '@/api/fleet';
import type { FleetTask, FleetTaskStatus } from '@/api/fleet';
import { cn } from '@/lib/utils';

/**
 * Max prompt preview length in Unicode code points. Display tuning
 * constant — lives next to the truncation it tunes.
 */
const PROMPT_MAX_POINTS = 80;

/**
 * Truncate a prompt by code point, never by UTF-16 unit. A naive
 * `String.slice` counts UTF-16 code units and can split a surrogate pair
 * (CJK Extension B, emoji), rendering a replacement glyph — the FE
 * sibling of the byte-slice panic in issue 2138.
 */
function truncatePrompt(prompt: string): string {
  const points = Array.from(prompt);
  if (points.length <= PROMPT_MAX_POINTS) return prompt;
  return `${points.slice(0, PROMPT_MAX_POINTS).join('')}…`;
}

/** Compact token count: `842`, `12.5k`, `1.2M`. */
function formatTokens(count: number): string {
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M`;
  if (count >= 1000) return `${(count / 1000).toFixed(1)}k`;
  return `${count}`;
}

/**
 * Validate a result-contract URL before rendering it as a link. The
 * callback payload is worker/LLM output — an untrusted trust boundary —
 * so only well-formed http(s) URLs become anchors; anything else
 * (`javascript:`, `data:`, malformed) falls back to plain text. Do not
 * rely on React's `javascript:` warning for this.
 */
function safeHttpUrl(raw: string): string | null {
  try {
    const url = new URL(raw);
    return url.protocol === 'http:' || url.protocol === 'https:' ? url.href : null;
  } catch {
    return null;
  }
}

/**
 * Read-only card for one dispatched fleet task in the worker-inbox rail.
 * Running cards show agent type, prompt preview, and status; terminal
 * cards add the result contract (branch, PR link, token usage, cost) and
 * failed / lost cards surface the error text. Not clickable — the only
 * interactive element is the external PR link.
 */
export function FleetTaskCard({ task }: { task: FleetTask }) {
  const totalTokens = (task.input_tokens ?? 0) + (task.output_tokens ?? 0);
  const hasTokens = task.input_tokens !== null || task.output_tokens !== null;
  const hasResultRow = Boolean(task.branch || task.pr_url) || hasTokens || task.cost_usd !== null;
  const prHref = task.pr_url === null ? null : safeHttpUrl(task.pr_url);
  // Succeeded tasks carry `error: null` per the result contract, so
  // "terminal with an error" is exactly the failed / lost surface.
  const showError = TERMINAL_STATUSES.includes(task.status) && Boolean(task.error);

  return (
    <div
      // Mirrors WorkerCard: `rounded-lg` (8px) + `px-2 py-1.5` keeps the
      // inner `rounded` (4px) badge concentric.
      className="flex w-full flex-col gap-1 rounded-lg border border-border bg-card px-2 py-1.5 text-[11px] text-foreground"
    >
      <div className="flex items-center justify-between gap-2">
        <span className="truncate font-medium">{task.agent_type}</span>
        <StatusBadge status={task.status} />
      </div>
      <p className="truncate text-[10px] text-muted-foreground" title={task.prompt}>
        {truncatePrompt(task.prompt)}
      </p>
      {hasResultRow && (
        // `tabular-nums` so polling refreshes don't reflow the figures.
        <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[10px] tabular-nums text-muted-foreground">
          {task.branch && (
            <span className="truncate font-mono" title={task.branch}>
              {task.branch}
            </span>
          )}
          {prHref !== null && (
            <a
              href={prHref}
              target="_blank"
              rel="noreferrer"
              // Negative margin + padding grows the tap target (#16)
              // without shifting the row's visual rhythm.
              className="-mx-1 -my-1.5 inline-flex shrink-0 items-center gap-0.5 rounded px-1 py-1.5 text-info hover:underline"
            >
              PR
              <ExternalLink className="h-2.5 w-2.5" aria-hidden />
            </a>
          )}
          {prHref === null && task.pr_url !== null && (
            // Untrusted / non-http(s) pr_url: show it, never link it.
            <span className="truncate" title={task.pr_url}>
              {task.pr_url}
            </span>
          )}
          {hasTokens && <span className="shrink-0">{formatTokens(totalTokens)} tok</span>}
          {task.cost_usd !== null && <span className="shrink-0">${task.cost_usd.toFixed(2)}</span>}
        </div>
      )}
      {showError && <p className="line-clamp-2 text-[10px] text-destructive">{task.error}</p>}
      <span className="text-[10px] tabular-nums text-muted-foreground">
        {task.completed_at
          ? `completed ${formatRelativeTime(task.completed_at)}`
          : `created ${formatRelativeTime(task.created_at)}`}
      </span>
    </div>
  );
}

function StatusBadge({ status }: { status: FleetTaskStatus }) {
  // Same chip treatment as WorkerCard's badge; `lost` is dimmed
  // destructive so it reads as failure-adjacent but distinct.
  const styles: Record<FleetTaskStatus, string> = {
    queued: 'bg-muted text-muted-foreground',
    running: 'bg-info text-white',
    succeeded: 'bg-success text-white',
    failed: 'bg-destructive text-white',
    lost: 'bg-destructive/60 text-white',
  };
  return (
    <span
      className={cn(
        'shrink-0 rounded px-1.5 py-px text-[9px] font-medium uppercase tracking-wide',
        styles[status],
      )}
    >
      {status}
    </span>
  );
}
