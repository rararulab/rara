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
 * Polled fetch of fleet tasks from `GET /api/v1/fleet/tasks` for the
 * worker-inbox fleet section. Polling (not WebSocket push) is the
 * recorded decision for the first slice — see
 * `specs/issue-2197-fleet-inbox-ui.spec.md` Decisions.
 */

import { useQuery } from '@tanstack/react-query';

import { fetchFleetTasks } from '@/api/fleet';

const FLEET_TASKS_QUERY_KEY = ['topology', 'fleet-tasks'] as const;

/** Poll interval. Fleet tasks are minutes-long coding jobs, so a 10s
 *  cadence is responsive without hammering the backend. Mechanism
 *  constant — stays here, not in YAML (docs/guides/anti-patterns.md). */
const POLL_MS = 10_000;

/** React-query hook polling the fleet task list. */
export function useFleetTasks() {
  return useQuery({
    queryKey: FLEET_TASKS_QUERY_KEY,
    queryFn: ({ signal }) => fetchFleetTasks({ signal }),
    refetchInterval: POLL_MS,
  });
}
