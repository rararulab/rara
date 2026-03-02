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

import { api } from './client';
import type { SkillSummary, SkillDetail, CreateSkillRequest } from './types';

export async function listSkills(): Promise<SkillSummary[]> {
  return api.get<SkillSummary[]>('/api/v1/skills');
}

export async function getSkill(name: string): Promise<SkillDetail> {
  return api.get<SkillDetail>(`/api/v1/skills/${encodeURIComponent(name)}`);
}

export async function createSkill(skill: CreateSkillRequest): Promise<SkillDetail> {
  return api.post<SkillDetail>('/api/v1/skills', skill);
}

export async function deleteSkill(name: string): Promise<void> {
  await api.del(`/api/v1/skills/${encodeURIComponent(name)}`);
}
