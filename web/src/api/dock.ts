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

import { api } from "@/api/client";

// ---------------------------------------------------------------------------
// Types matching rara-dock models
// ---------------------------------------------------------------------------

export type Actor = "human" | "agent";

export interface DockBlock {
  id: string;
  block_type: string;
  html: string;
  diff?: DockDiff | null;
}

export interface DockDiff {
  original: string;
  modified: string;
  author: Actor;
}

export interface DockFact {
  id: string;
  content: string;
  source: Actor;
}

export interface DockSelection {
  start: number;
  end: number;
  text: string;
}

export interface DockAnnotation {
  id: string;
  block_id: string;
  content: string;
  author: Actor;
  anchor_y: number;
  timestamp: number;
  selection?: DockSelection | null;
}

export interface DockMutation {
  op: string;
  actor: Actor;
  block?: DockBlock;
  fact?: DockFact;
  annotation?: DockAnnotation;
  id?: string;
}

export interface DockSessionMeta {
  id: string;
  title: string;
  preview: string;
  created_at: number;
  updated_at: number;
  selected_anchor?: string | null;
}

export interface DockHistoryEntry {
  id: string;
  anchor_name: string;
  timestamp: string;
  label: string;
  preview: string;
  state: Record<string, unknown>;
  is_selected: boolean;
}

// ---------------------------------------------------------------------------
// API response types
// ---------------------------------------------------------------------------

export interface DockBootstrapResponse {
  sessions: DockSessionMeta[];
  active_session_id?: string | null;
}

export interface DockSessionResponse {
  session: DockSessionMeta;
  annotations: DockAnnotation[];
  history: DockHistoryEntry[];
  selected_anchor?: string | null;
  blocks: DockBlock[];
  facts: DockFact[];
}

export interface DockTurnRequest {
  session_id: string;
  content: string;
  is_command: boolean;
  blocks: DockBlock[];
  facts: DockFact[];
  annotations: DockAnnotation[];
  selected_anchor?: string | null;
}

export interface DockTurnResponse {
  session_id: string;
  reply: string;
  mutations: DockMutation[];
  history: DockHistoryEntry[];
  selected_anchor?: string | null;
  session?: DockSessionMeta | null;
  annotations: DockAnnotation[];
  blocks: DockBlock[];
  facts: DockFact[];
}

export interface DockSessionDocument {
  session: DockSessionMeta;
  annotations: DockAnnotation[];
  facts: DockFact[];
}

// ---------------------------------------------------------------------------
// API functions
// ---------------------------------------------------------------------------

export function dockBootstrap(): Promise<DockBootstrapResponse> {
  return api.get<DockBootstrapResponse>("/api/dock/bootstrap");
}

export function dockGetSession(
  sessionId: string,
  selectedAnchor?: string,
): Promise<DockSessionResponse> {
  const params = new URLSearchParams({ session_id: sessionId });
  if (selectedAnchor) {
    params.set("selected_anchor", selectedAnchor);
  }
  return api.get<DockSessionResponse>(
    `/api/dock/session?${params.toString()}`,
  );
}

export function dockCreateSession(
  title?: string,
): Promise<DockSessionMeta> {
  return api.post<DockSessionMeta>("/api/dock/sessions", { title });
}

export function dockMutateSession(
  sessionId: string,
  mutations: DockMutation[],
): Promise<DockSessionDocument> {
  return api.post<DockSessionDocument>(
    `/api/dock/sessions/${sessionId}/mutate`,
    { mutations },
  );
}

export function dockTurn(
  request: DockTurnRequest,
): Promise<DockTurnResponse> {
  return api.post<DockTurnResponse>("/api/dock/turn", request);
}

export function dockUpdateWorkspace(
  activeSessionId: string,
): Promise<void> {
  return api.patch<void>("/api/dock/workspace", {
    active_session_id: activeSessionId,
  });
}
