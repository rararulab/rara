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

/** Cascade entry classification. */
export type CascadeEntryKind = "user_input" | "thought" | "action" | "observation";

/** A single entry in the cascade trace. */
export interface CascadeEntry {
  id: string;
  kind: CascadeEntryKind;
  content: string;
  timestamp: string;
  metadata?: Record<string, unknown>;
}

/** One reasoning-action cycle within a turn. */
export interface CascadeTick {
  index: number;
  entries: CascadeEntry[];
}

/** Aggregate statistics for the cascade trace. */
export interface CascadeSummary {
  tick_count: number;
  tool_call_count: number;
  total_entries: number;
}

/** A complete cascade trace for one agent turn. */
export interface CascadeTrace {
  message_id: string;
  ticks: CascadeTick[];
  summary: CascadeSummary;
}
