/*
 * Copyright 2025 Crrow
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
import type { Skill } from './types';

export async function listSkills(): Promise<Skill[]> {
  return api.get<Skill[]>('/api/v1/skills');
}

export async function getSkill(name: string): Promise<Skill> {
  return api.get<Skill>(`/api/v1/skills/${encodeURIComponent(name)}`);
}

export async function createSkill(skill: Omit<Skill, 'enabled'>): Promise<Skill> {
  return api.post<Skill>('/api/v1/skills', skill);
}

export async function updateSkill(name: string, updates: Partial<Skill>): Promise<Skill> {
  return api.put<Skill>(`/api/v1/skills/${encodeURIComponent(name)}`, updates);
}

export async function deleteSkill(name: string): Promise<void> {
  await api.del(`/api/v1/skills/${encodeURIComponent(name)}`);
}
