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
 * Typed client for the fleet task read API (`GET /api/v1/fleet/tasks`,
 * issue 2195). The wire shape mirrors the `fleet_tasks` record defined in
 * issue 2194's migration, minus `callback_token` — the read API never
 * exposes the per-task webhook secret.
 */

import { api } from './client';

/** Lifecycle states of a fleet task — TEXT wire form of the backend's
 *  `FleetTaskStatus` enum (issue 2194). */
export type FleetTaskStatus = 'queued' | 'running' | 'succeeded' | 'failed' | 'lost';

/** Statuses after which the result contract fields are populated. */
export const TERMINAL_STATUSES: readonly FleetTaskStatus[] = ['succeeded', 'failed', 'lost'];

/**
 * One dispatched fleet task. Result-contract fields (`branch`,
 * `commit_sha`, `pr_url`, token counts, `cost_usd`, `error`, `exit_code`)
 * are `null` until the task reaches a terminal status.
 */
export interface FleetTask {
  id: string;
  agent_type: string;
  status: FleetTaskStatus;
  backend: string;
  prompt: string;
  repo_url: string;
  session_key: string | null;
  branch: string | null;
  commit_sha: string | null;
  pr_url: string | null;
  input_tokens: number | null;
  output_tokens: number | null;
  cost_usd: number | null;
  error: string | null;
  exit_code: number | null;
  created_at: string;
  started_at: string | null;
  completed_at: string | null;
}

/** Fetch every fleet task, newest first (backend orders by `created_at`). */
export function fetchFleetTasks(options?: { signal?: AbortSignal }): Promise<FleetTask[]> {
  return api.get<FleetTask[]>('/api/v1/fleet/tasks', options);
}
