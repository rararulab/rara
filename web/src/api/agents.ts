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
import type { AgentResponse, CreateAgentRequest } from './types';

export async function fetchAgents(): Promise<AgentResponse[]> {
  return api.get<AgentResponse[]>('/api/v1/agents');
}

export async function fetchAgent(name: string): Promise<AgentResponse> {
  return api.get<AgentResponse>(`/api/v1/agents/${encodeURIComponent(name)}`);
}

export async function createAgent(req: CreateAgentRequest): Promise<AgentResponse> {
  return api.post<AgentResponse>('/api/v1/agents', req);
}

export async function deleteAgent(name: string): Promise<void> {
  await api.del(`/api/v1/agents/${encodeURIComponent(name)}`);
}
