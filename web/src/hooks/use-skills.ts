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
 * Cached fetch of the available skills from `GET /api/v1/skills`.
 *
 * The mention picker uses this to populate `@skill-name` autocomplete
 * inside the prompt editor.
 */

import { useQuery } from '@tanstack/react-query';

import { api } from '@/api/client';

/** Wire shape of `GET /api/v1/skills` — keep in sync with
 *  `crates/extensions/backend-admin/src/skills/router.rs::SkillSummary`. */
export interface SkillSummary {
  name: string;
  description: string;
  allowed_tools: string[];
  source: string | null;
  homepage: string | null;
  license: string | null;
  eligible: boolean;
}

const SKILLS_QUERY_KEY = ['topology', 'skills'] as const;

/** Skills change rarely — refetch on focus is fine, no polling. */
const STALE_MS = 60 * 1000;

export function useSkills() {
  return useQuery({
    queryKey: SKILLS_QUERY_KEY,
    queryFn: () => api.get<SkillSummary[]>('/api/v1/skills'),
    staleTime: STALE_MS,
  });
}
