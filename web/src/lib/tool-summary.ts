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

/**
 * Extract a short human-readable summary from tool arguments for the
 * agent-live card's collapsed row.
 *
 * Contract (from issue #1615): `description` wins when present, falling
 * back to `query`, `file_path`/`path`, then `command` (each capped).
 * This is intentionally narrower than the historical
 * `timeline-colors::eventSummary` which served the kernel timeline
 * panel and favoured `query` as the first pick — the two surfaces
 * disagree about priority, so this helper is separate rather than a
 * shared one with subtle branching.
 */

/** Max characters kept for the short-form summary. */
const SUMMARY_MAX = 100;

/** Max characters kept for the `description` field specifically. */
const DESCRIPTION_MAX = 120;

/**
 * Pick the most informative short string for a tool invocation.
 *
 * Priority (spec): `description` → `query` → `file_path`/`path` (shortened)
 * → `command` (capped).
 */
export function toolSummary(input: Record<string, unknown> | null | undefined): string {
  if (!input) return '';

  const description = readString(input, 'description');
  if (description) return cap(description, DESCRIPTION_MAX);

  const query = readString(input, 'query');
  if (query) return cap(query, SUMMARY_MAX);

  const filePath = readString(input, 'file_path') ?? readString(input, 'path');
  if (filePath) return shortenPath(filePath);

  const command = readString(input, 'command');
  if (command) return cap(command, SUMMARY_MAX);

  return '';
}

function readString(obj: Record<string, unknown>, key: string): string | null {
  const v = obj[key];
  return typeof v === 'string' && v.length > 0 ? v : null;
}

function cap(s: string, max: number): string {
  return s.length > max ? `${s.slice(0, max)}...` : s;
}

/** Collapse a long path to `.../parent/basename`. */
function shortenPath(p: string): string {
  const parts = p.split('/');
  if (parts.length <= 3) return p;
  return `.../${parts.slice(-2).join('/')}`;
}
