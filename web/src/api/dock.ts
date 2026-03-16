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

/** SSE event received during a dock turn. */
export type DockTurnEvent =
  | { type: "text_delta"; text: string }
  | { type: "tool_call_start"; name: string; id: string; arguments: unknown }
  | {
      type: "tool_call_end";
      id: string;
      result_preview: string;
      success: boolean;
      error?: string | null;
    }
  | { type: "dock_turn_complete"; data: DockTurnResponse }
  | { type: "error"; error: string }
  | { type: "done" };

/**
 * Execute a dock turn via SSE streaming.
 *
 * The endpoint returns an SSE stream of events.  The `onEvent` callback is
 * invoked for each event.  Returns a promise that resolves when the stream
 * ends.
 */
export async function dockTurnStream(
  request: DockTurnRequest,
  onEvent: (event: DockTurnEvent) => void,
): Promise<void> {
  const response = await fetch("/api/dock/turn", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(request),
  });

  if (!response.ok) {
    const text = await response.text();
    throw new Error(`Dock turn failed (${response.status}): ${text}`);
  }

  // If the response is not SSE (e.g. test mode returns JSON), handle it.
  const contentType = response.headers.get("content-type") ?? "";
  if (contentType.includes("application/json")) {
    const data = (await response.json()) as DockTurnResponse;
    onEvent({ type: "dock_turn_complete", data });
    onEvent({ type: "done" });
    return;
  }

  // Parse the SSE stream.
  const reader = response.body?.getReader();
  if (!reader) return;

  const decoder = new TextDecoder();
  let buffer = "";

  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;

    buffer += decoder.decode(value, { stream: true });
    const lines = buffer.split("\n");
    buffer = lines.pop() ?? "";

    let currentEvent = "";
    let currentData = "";

    for (const line of lines) {
      if (line.startsWith("event: ")) {
        currentEvent = line.slice(7).trim();
      } else if (line.startsWith("data: ")) {
        currentData += line.slice(6);
      } else if (line === "" && currentEvent) {
        // Empty line = end of SSE event
        try {
          if (currentEvent === "done") {
            onEvent({ type: "done" });
          } else if (currentEvent === "text_delta") {
            const parsed = JSON.parse(currentData);
            onEvent({ type: "text_delta", text: parsed.text });
          } else if (currentEvent === "tool_call_start") {
            const parsed = JSON.parse(currentData);
            onEvent({
              type: "tool_call_start",
              name: parsed.name,
              id: parsed.id,
              arguments: parsed.arguments,
            });
          } else if (currentEvent === "tool_call_end") {
            const parsed = JSON.parse(currentData);
            onEvent({
              type: "tool_call_end",
              id: parsed.id,
              result_preview: parsed.result_preview,
              success: parsed.success,
              error: parsed.error,
            });
          } else if (currentEvent === "dock_turn_complete") {
            const parsed = JSON.parse(currentData) as DockTurnResponse;
            onEvent({ type: "dock_turn_complete", data: parsed });
          } else if (currentEvent === "error") {
            const parsed = JSON.parse(currentData);
            onEvent({ type: "error", error: parsed.error ?? "unknown error" });
          }
        } catch {
          // Skip malformed events
        }
        currentEvent = "";
        currentData = "";
      }
    }
  }
}

/** Non-streaming dock turn (kept for backwards compatibility in tests). */
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
